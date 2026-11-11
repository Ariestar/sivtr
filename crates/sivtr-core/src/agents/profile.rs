//! One registry for every native provider, in the role magpie's `parserFor`
//! plays: discovery and parsing go through here, and each provider declares
//! exactly one current format.

use std::path::Path;

use anyhow::Result;

use super::{
    claude, cmdc, codex, cursor, dsh, gemini, generic, goose, grok, hermes, openclaw, opencode, pi,
    qoder, qwen, zcode, AgentProvider, AgentSession, SessionListing,
};

type ListFn = fn(AgentProvider, Option<&Path>) -> Result<SessionListing>;
type ParseFn = fn(AgentProvider, &Path) -> Result<AgentSession>;

struct NativeProfile {
    provider: AgentProvider,
    /// Several transcript files share one logical session id.
    merge_siblings: bool,
    list: ListFn,
    parse: ParseFn,
}

const NATIVE: &[NativeProfile] = &[
    NativeProfile {
        provider: AgentProvider::Claude,
        merge_siblings: false,
        list: claude::list_outcome,
        parse: claude::parse_file,
    },
    NativeProfile {
        provider: AgentProvider::Codex,
        merge_siblings: true,
        list: codex::list_outcome,
        parse: codex::parse_file,
    },
    NativeProfile {
        provider: AgentProvider::CommandCode,
        merge_siblings: false,
        list: cmdc::list_outcome,
        parse: cmdc::parse_file,
    },
    NativeProfile {
        provider: AgentProvider::Cursor,
        merge_siblings: false,
        list: cursor::list_outcome,
        parse: cursor::parse_file,
    },
    NativeProfile {
        provider: AgentProvider::Dsh,
        merge_siblings: false,
        list: dsh::list_outcome,
        parse: dsh::parse_file,
    },
    NativeProfile {
        provider: AgentProvider::Gemini,
        merge_siblings: false,
        list: gemini::list_outcome,
        parse: gemini::parse_file,
    },
    NativeProfile {
        provider: AgentProvider::Goose,
        merge_siblings: false,
        list: goose::list_outcome,
        parse: goose::parse_file,
    },
    NativeProfile {
        provider: AgentProvider::Grok,
        merge_siblings: false,
        list: grok::list_outcome,
        parse: grok::parse_file,
    },
    NativeProfile {
        provider: AgentProvider::Hermes,
        merge_siblings: false,
        list: hermes::list_outcome,
        parse: hermes::parse_file,
    },
    NativeProfile {
        provider: AgentProvider::OpenClaw,
        merge_siblings: false,
        list: openclaw::list_outcome,
        parse: openclaw::parse_file,
    },
    NativeProfile {
        provider: AgentProvider::OpenCode,
        merge_siblings: false,
        list: opencode::list_outcome,
        parse: opencode::parse_file,
    },
    NativeProfile {
        provider: AgentProvider::Pi,
        merge_siblings: false,
        list: pi::list_outcome,
        parse: pi::parse_file,
    },
    NativeProfile {
        provider: AgentProvider::Qoder,
        merge_siblings: false,
        list: qoder::list_outcome,
        parse: qoder::parse_file,
    },
    NativeProfile {
        provider: AgentProvider::QoderCn,
        merge_siblings: false,
        list: qoder::list_cn_outcome,
        parse: qoder::parse_file,
    },
    NativeProfile {
        provider: AgentProvider::Qwen,
        merge_siblings: false,
        list: qwen::list_outcome,
        parse: qwen::parse_file,
    },
    NativeProfile {
        provider: AgentProvider::Zcode,
        merge_siblings: false,
        list: zcode::list_outcome,
        parse: zcode::parse_file,
    },
];

fn native(provider: AgentProvider) -> Option<&'static NativeProfile> {
    NATIVE.iter().find(|profile| profile.provider == provider)
}

/// A provider whose current format this build can read.
pub fn is_registered(provider: AgentProvider) -> bool {
    native(provider).is_some() || generic::is_supported(provider)
}

pub fn merges_siblings(provider: AgentProvider) -> bool {
    native(provider).is_some_and(|profile| profile.merge_siblings)
}

/// List one provider. An unregistered provider contributes nothing.
pub fn list(provider: AgentProvider, cwd: Option<&Path>) -> Result<SessionListing> {
    if let Some(profile) = native(provider) {
        return (profile.list)(provider, cwd);
    }
    if generic::is_supported(provider) {
        return generic::list_provider(provider, cwd);
    }
    Ok((Vec::new(), Vec::new()))
}

/// Parse one native file with that provider's current parser.
pub fn parse(provider: AgentProvider, path: &Path) -> Result<AgentSession> {
    if let Some(profile) = native(provider) {
        return (profile.parse)(provider, path);
    }
    generic::parse_provider(provider, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn each_native_provider_is_registered_once() {
        let mut seen = HashSet::new();
        for spec in AgentProvider::all() {
            let provider = spec.provider;
            let dedicated = native(provider).is_some();
            let declared = generic::is_supported(provider);
            assert!(
                !(dedicated && declared),
                "{} is both a dedicated profile and a generic profile",
                provider.command_name()
            );
            if dedicated || declared {
                assert!(seen.insert(provider));
            }
        }
        assert_eq!(NATIVE.len(), 16);
        assert!(merges_siblings(AgentProvider::Codex));
        assert!(!merges_siblings(AgentProvider::Claude));
        assert!(!is_registered(AgentProvider::Amp));
    }
}
