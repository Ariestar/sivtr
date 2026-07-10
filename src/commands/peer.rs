use anyhow::{bail, Result};

use crate::cli::{PeerAction, PeerCommand};
use crate::output;
use crate::remote::local;
use crate::remote::protocol::{LocalRequest, LocalResponse};

pub fn execute(command: PeerCommand) -> Result<()> {
    match command.action {
        PeerAction::List => list(),
        PeerAction::Forget { peer } => forget(&peer),
    }
}

fn list() -> Result<()> {
    match local::call(LocalRequest::PeerList)? {
        LocalResponse::Peers(peers) => {
            if peers.is_empty() {
                output::plain("no peers known");
            }
            for peer in peers {
                output::detail(peer.name, peer.id);
            }
            Ok(())
        }
        response => bail!("Unexpected daemon response: {response:?}"),
    }
}

fn forget(peer: &str) -> Result<()> {
    match local::call(LocalRequest::PeerForget {
        peer: peer.to_string(),
    })? {
        LocalResponse::Peer(peer) => {
            output::success(format!("forgot peer `{}`", peer.name));
            Ok(())
        }
        response => bail!("Unexpected daemon response: {response:?}"),
    }
}
