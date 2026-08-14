//! Group query fan-out: resolve the (member, share) targets a group scope
//! addresses, then run the query across every target and merge the results
//! qualified per member. Pure read path — no membership or roster changes.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Result};
use tokio::task::JoinSet;

use crate::commands::memory::filter::{self, Filter};

use super::context::DaemonContext;
use super::net;
use super::protocol::{
    qualify_query_scope, GroupQueryResponse, QueryResponse, RemoteRequest, RemoteResponse,
};
use super::state::{GroupInfo, GroupMemberInfo, StateStore};

/// Per-share budget for one remote dial inside a fan-out.
const PER_SHARE_QUERY_TIMEOUT: Duration = Duration::from_millis(2500);

/// One (member, share) pair a group query fans out to, flattened from the
/// roster and pinned by the three-segment scopes `team/alice` and
/// `team/alice/proj-b`. The local member's targets run in-process; every
/// other member's are dialed over the wire.
#[derive(Debug)]
pub(crate) struct FanOutTarget {
    pub peer_id: String,
    pub peer_name: String,
    pub share_id: String,
    pub share_name: String,
}

/// Resolve which (member, share) pairs a group query fans out to. The caller's
/// own contribution is a target like any other member's - the local member is
/// queried in-process by [`group_fan_out`]. `member` pins one member by
/// display name or peer id; `share` pins one contributed share per member.
pub(crate) fn group_targets(
    store: &StateStore,
    group: &GroupInfo,
    member: Option<&str>,
    share: Option<&str>,
) -> Result<Vec<FanOutTarget>> {
    let all: Vec<GroupMemberInfo> = store.members(&group.id)?;
    let rows: Vec<GroupMemberInfo> = match member {
        Some(name) => {
            let needle = name.to_ascii_lowercase();
            let matches: Vec<GroupMemberInfo> = all
                .into_iter()
                .filter(|row| row.peer_name.to_ascii_lowercase() == needle || row.peer_id == needle)
                .collect();
            if matches.is_empty() {
                bail!("No group member named `{name}` in `{}`", group.name);
            }
            matches
        }
        None => all,
    };
    let mut targets = Vec::new();
    for row in rows {
        for contribution in store.group_shares(&group.id, &row.peer_id)? {
            if let Some(pinned) = share {
                if !contribution.share_name.eq_ignore_ascii_case(pinned) {
                    continue;
                }
            }
            targets.push(FanOutTarget {
                peer_id: row.peer_id.clone(),
                peer_name: row.peer_name.clone(),
                share_id: contribution.share_id,
                share_name: contribution.share_name,
            });
        }
    }
    // `team/alice/proj-b` pins one contributed share per member.
    if let Some(share_name) = share {
        if targets.is_empty() {
            bail!(
                "No member contributes a share named `{share_name}` in `{}`",
                group.name
            );
        }
    }
    Ok(targets)
}

/// Per-share query bounds for a group fan-out: relevance needs the merged
/// corpus to score, so `rank` (and `latest`/`limit`) stay stripped there;
/// recency and limit bounds compose across shares and are pushed down to
/// bound each member's response size.
fn per_share_bounds(full: &Filter) -> Filter {
    let mut bounds = full.clone();
    bounds.rank = None;
    if full.rank.is_some() {
        bounds.latest = None;
        bounds.limit = None;
    }
    bounds
}

/// Fan out a group query: the caller's own contributions run in-process (a
/// failure is a real error), every remote (member, share) is dialed in parallel
/// under a per-share budget, and results are merged qualified per member and
/// share. Members that did not answer are reported as skipped.
pub(crate) async fn group_fan_out(
    context: &Arc<DaemonContext>,
    group_name: &str,
    targets: &[FanOutTarget],
    source: &str,
    filter: Filter,
) -> Result<GroupQueryResponse> {
    // Shares only bound the set (pattern/status/time/...). For non-relevance
    // ordering, `latest`/`limit` compose across shares: each share's top-N is
    // a superset of the global top-N's per-share part, so pushing them down
    // bounds the per-member wire cost without changing the merged result.
    // Relevance (BM25) is ranked only after the group-wide merge, so it stays
    // unbounded per share and the full bounds are applied once below.
    let full = filter.for_remote_peer();
    let bounds = per_share_bounds(&full);

    let self_id = context.identity.id();
    let mut records = Vec::new();
    let mut anchors = Vec::new();
    // Every result is scoped `team/<peer-id>/proj-b` so members stay apart and
    // records round-trip through show/zoom/nav. The member segment is the
    // stable peer id, not the display name: two devices can share a hostname
    // (and a workspace name), and a name-based scope would collide and make
    // the refs ambiguous.
    let mut merge = |peer_id: &str, share_name: &str, mut query: QueryResponse| {
        qualify_query_scope(&format!("{group_name}/{peer_id}/{share_name}"), &mut query);
        records.extend(query.records);
        anchors.extend(query.anchors);
    };

    // The local member's contributions are part of the group, so they are
    // queried like any other share - just in-process instead of over the wire.
    // Local failures propagate before any remote dial; they are not "offline".
    for target in targets {
        if target.peer_id != self_id {
            continue;
        }
        let share = context.store.share(&target.share_id)?;
        let query = tokio::task::spawn_blocking({
            let root = share.root.clone();
            let redact = share.redact;
            let source = source.to_string();
            let filter = bounds.clone();
            move || {
                let (records, anchors) = crate::commands::memory::workset::run_on_share(
                    std::path::Path::new(&root),
                    &source,
                    filter,
                    redact,
                )?;
                Ok::<_, anyhow::Error>(QueryResponse { records, anchors })
            }
        })
        .await??;
        merge(&target.peer_id, &target.share_name, query);
    }

    let mut tasks = JoinSet::new();
    for target in targets {
        if target.peer_id == self_id {
            continue;
        }
        let context = context.clone();
        let source = source.to_string();
        let filter = bounds.clone();
        let peer_id = target.peer_id.clone();
        let share_id = target.share_id.clone();
        let share_name = target.share_name.clone();
        tasks.spawn(async move {
            let result = tokio::time::timeout(
                PER_SHARE_QUERY_TIMEOUT,
                net::exchange_with_peer(
                    &context.store,
                    &context.endpoint,
                    &peer_id,
                    RemoteRequest::Query {
                        share_id,
                        source,
                        filter,
                    },
                ),
            )
            .await;
            (peer_id, share_name, result)
        });
    }
    // A member is online when any of its shares answered, decided only after
    // every share task completes so a later failure cannot reclassify a peer
    // whose records were already merged.
    let mut online: HashSet<String> = HashSet::new();
    while let Some(joined) = tasks.join_next().await {
        let Ok((peer_id, share_name, result)) = joined else {
            continue;
        };
        if let Ok(Ok(RemoteResponse::Query(query))) = result {
            merge(&peer_id, &share_name, query);
            online.insert(peer_id);
        }
    }
    // Distinct offline members by display name, in roster order.
    let mut skipped: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for target in targets {
        if target.peer_id == self_id || online.contains(&target.peer_id) {
            continue;
        }
        if seen.insert(target.peer_name.clone()) {
            skipped.push(target.peer_name.clone());
        }
    }

    // Order the merged corpus once, as one group: `latest` window -> sort ->
    // `limit`. Re-running the pipeline here is idempotent for the bounds each
    // share already applied; the shared code also ranks the merged corpus as
    // a whole when the sort is relevance.
    let merged = filter::apply(PathBuf::new(), records, anchors, full)?;
    Ok(GroupQueryResponse {
        query: QueryResponse {
            records: merged.records,
            anchors: merged.anchors,
        },
        skipped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Group with the owner contributing `project` and a second member `bob`
    /// contributing `bob-ws`, so member pinning and share pinning each have a
    /// live target.
    fn group_with_members() -> (tempfile::TempDir, StateStore, String) {
        let temp = tempfile::tempdir().expect("temp dir");
        let workspace = temp.path().join("workspace");
        std::fs::create_dir(&workspace).expect("workspace dir");
        let store = StateStore::open(temp.path().join("state.db")).expect("store");
        let share = store
            .add_share("workspace-key", &workspace, "project", true)
            .expect("share");
        let self_id = iroh::SecretKey::generate().public().to_string();
        let bob_id = iroh::SecretKey::generate().public().to_string();
        store.add_group("team", &self_id, "self").expect("group");
        store
            .add_group_share("team", &self_id, &share.id, &share.name)
            .expect("contribution");
        store.save_remote_peer(&bob_id, "bob", "{}").expect("peer");
        store.add_member("team", &bob_id, "member").expect("member");
        let bob_share = store
            .add_share("workspace-bob", &workspace, "bob-ws", true)
            .expect("share");
        store
            .add_group_share("team", &bob_id, &bob_share.id, &bob_share.name)
            .expect("bob contribution");
        (temp, store, self_id)
    }

    #[test]
    fn group_targets_include_the_local_member() {
        let (_temp, store, self_id) = group_with_members();
        let group = store.group("team").expect("group");
        let targets = group_targets(&store, &group, None, None).expect("targets");
        assert!(
            targets.iter().any(|target| target.peer_id == self_id),
            "the caller's own contribution is a fan-out target"
        );
        assert!(targets.iter().any(|target| target.peer_name == "bob"));
    }

    #[test]
    fn group_targets_pin_one_member_by_name_or_id() {
        let (_temp, store, self_id) = group_with_members();
        let group = store.group("team").expect("group");
        let targets = group_targets(&store, &group, Some("self"), None).expect("targets");
        assert_eq!(targets.len(), 1);
        assert_eq!(
            targets[0].peer_id, self_id,
            "self-query resolves to the local member"
        );
        let error =
            group_targets(&store, &group, Some("nobody"), None).expect_err("unknown member");
        assert!(error.to_string().contains("No group member named"));
    }

    #[test]
    fn group_targets_pin_one_contributed_share() {
        let (_temp, store, _self_id) = group_with_members();
        let group = store.group("team").expect("group");
        let targets = group_targets(&store, &group, None, Some("project")).expect("targets");
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].peer_name, "self");
        assert_eq!(targets[0].share_name, "project");

        let error =
            group_targets(&store, &group, None, Some("missing")).expect_err("unknown share");
        assert!(error.to_string().contains("No member contributes a share"));
    }

    #[test]
    fn per_share_bounds_push_down_recency_but_not_relevance() {
        let recency = Filter {
            latest: Some(5),
            limit: Some(10),
            rank: None,
            ..Filter::default()
        };
        let bounds = per_share_bounds(&recency);
        assert_eq!(bounds.rank, None);
        assert_eq!(bounds.latest, Some(5));
        assert_eq!(bounds.limit, Some(10));

        let relevance = Filter {
            latest: Some(5),
            limit: Some(10),
            rank: Some("query".to_string()),
            ..Filter::default()
        };
        let bounds = per_share_bounds(&relevance);
        assert_eq!(bounds.rank, None);
        assert_eq!(bounds.latest, None);
        assert_eq!(bounds.limit, None);
    }
}
