//! Custom session storage: `FileSession` (implements `Session`) that holds a
//! serializable `SessionData`, persisted in a binary file (bincode).

use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::sync::Mutex;

use futures_core::future::BoxFuture;
use grammers_session::types::{DcOption, PeerId, PeerInfo, UpdateState, UpdatesState};
use grammers_session::{Session, SessionData};
use serde::{Deserialize, Serialize};

/// In-memory serializable session, driven by our own persistence.
#[derive(Default)]
pub struct FileSession {
    data: Mutex<SessionData>,
}

/// Persisted representation: `SessionData` is not serializable as-is, but all
/// of its public fields are. We serialize them through a wrapper.
#[derive(Serialize, Deserialize)]
struct PersistedSession {
    home_dc: i32,
    dc_options: HashMap<i32, DcOption>,
    peer_infos: HashMap<PeerId, PeerInfo>,
    updates_state: UpdatesState,
}

impl FileSession {
    /// A pristine session with the default known datacenters.
    pub fn new() -> Self {
        Self::default()
    }

    /// Full state (auth keys included) as a `SessionData`.
    pub fn snapshot(&self) -> SessionData {
        let d = &self.data.lock().unwrap();
        SessionData {
            home_dc: d.home_dc,
            dc_options: d.dc_options.clone(),
            peer_infos: d.peer_infos.clone(),
            updates_state: d.updates_state.clone(),
        }
    }

    /// Restores a previously saved state.
    pub fn restore(&self, data: SessionData) {
        *self.data.lock().unwrap() = data;
    }
}

impl Session for FileSession {
    fn home_dc_id(&self) -> i32 {
        self.data.lock().unwrap().home_dc
    }

    fn set_home_dc_id(&self, dc_id: i32) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.data.lock().unwrap().home_dc = dc_id;
        })
    }

    fn dc_option(&self, dc_id: i32) -> Option<DcOption> {
        self.data.lock().unwrap().dc_options.get(&dc_id).cloned()
    }

    fn set_dc_option(&self, dc_option: &DcOption) -> BoxFuture<'_, ()> {
        let dc_option = dc_option.clone();
        Box::pin(async move {
            self.data
                .lock()
                .unwrap()
                .dc_options
                .insert(dc_option.id, dc_option);
        })
    }

    fn peer(&self, peer: PeerId) -> BoxFuture<'_, Option<PeerInfo>> {
        Box::pin(async move { self.data.lock().unwrap().peer_infos.get(&peer).cloned() })
    }

    fn cache_peer(&self, peer: &PeerInfo) -> BoxFuture<'_, ()> {
        let peer = peer.clone();
        Box::pin(async move {
            self.data.lock().unwrap().peer_infos.insert(peer.id(), peer);
        })
    }

    fn updates_state(&self) -> BoxFuture<'_, UpdatesState> {
        Box::pin(async move { self.data.lock().unwrap().updates_state.clone() })
    }

    fn set_update_state(&self, update: UpdateState) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            let mut data = self.data.lock().unwrap();
            match update {
                UpdateState::All(updates_state) => data.updates_state = updates_state,
                UpdateState::Primary { pts, date, seq } => {
                    data.updates_state.pts = pts;
                    data.updates_state.date = date;
                    data.updates_state.seq = seq;
                }
                UpdateState::Secondary { qts } => {
                    data.updates_state.qts = qts;
                }
                UpdateState::Channel { id, pts } => {
                    data.updates_state
                        .channels
                        .retain(|c| c.id != id);
                    data.updates_state
                        .channels
                        .push(grammers_session::types::ChannelState { id, pts });
                }
            }
        })
    }
}

/// Loads a persisted session, or a pristine one if absent.
pub fn load_or_new(path: &Path) -> FileSession {
    match load(path) {
        Some(session) => session,
        None => FileSession::new(),
    }
}

/// Loads a session from a binary file.
pub fn load(path: &Path) -> Option<FileSession> {
    let bytes = std::fs::read(path).ok()?;
    let persisted: PersistedSession = bincode::deserialize(&bytes).ok()?;
    let session = FileSession::new();
    session.restore(SessionData {
        home_dc: persisted.home_dc,
        dc_options: persisted.dc_options,
        peer_infos: persisted.peer_infos,
        updates_state: persisted.updates_state,
    });
    Some(session)
}

/// Writes the session to disk (binary file, atomic write).
pub fn save(session: &FileSession, path: &Path) -> io::Result<()> {
    let snap = session.snapshot();
    let persisted = PersistedSession {
        home_dc: snap.home_dc,
        dc_options: snap.dc_options,
        peer_infos: snap.peer_infos,
        updates_state: snap.updates_state,
    };
    let bytes = bincode::serialize(&persisted)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, &bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_roundtrip_is_preserved() {
        let s1 = FileSession::new();
        let home_dc = s1.snapshot().home_dc;
        let bytes = bincode::serialize(&PersistedSession {
            home_dc,
            dc_options: s1.snapshot().dc_options,
            peer_infos: s1.snapshot().peer_infos,
            updates_state: s1.snapshot().updates_state,
        })
        .unwrap();

        let data: PersistedSession = bincode::deserialize(&bytes).unwrap();
        assert_eq!(data.home_dc, home_dc);
    }

    #[test]
    fn save_then_load_restores_the_session() {
        let dir = std::env::temp_dir().join(format!("tg-session-{}", std::process::id()));
        let path = dir.join("session.bin");
        std::fs::create_dir_all(&dir).unwrap();

        let s1 = FileSession::new();
        s1.restore(SessionData {
            home_dc: 2,
            ..Default::default()
        });
        save(&s1, &path).unwrap();

        let s2 = load(&path).unwrap();
        assert_eq!(s2.home_dc_id(), 2);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_absent_returns_none() {
        let path = std::env::temp_dir().join("tg-session-nope-absent.bin");
        assert!(load(&path).is_none());
    }
}