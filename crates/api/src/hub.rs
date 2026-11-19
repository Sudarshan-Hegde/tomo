//! Tracks who is connected over WebSocket so events can be pushed to them live.
//! In-memory, so it works for a single server instance (fine for the free tier).

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
};

use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tomo_shared::{ServerMsg, Uuid};

type Tx = UnboundedSender<ServerMsg>;

#[derive(Default)]
pub struct Hub {
    // A user can have several tabs or devices open at once.
    conns: Mutex<HashMap<Uuid, Vec<(u64, Tx)>>>,
    next_id: AtomicU64,
}

impl Hub {
    /// Returns (connection id, a sender for this connection only, the receiver to drain).
    pub fn register(&self, user: Uuid) -> (u64, Tx, UnboundedReceiver<ServerMsg>) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = unbounded_channel();
        self.conns.lock().unwrap().entry(user).or_default().push((id, tx.clone()));
        (id, tx, rx)
    }

    pub fn unregister(&self, user: Uuid, conn_id: u64) {
        let mut map = self.conns.lock().unwrap();
        if let Some(list) = map.get_mut(&user) {
            list.retain(|(id, _)| *id != conn_id);
            if list.is_empty() {
                map.remove(&user);
            }
        }
    }

    /// Push an event to every open connection of `user`. Offline users are skipped
    /// (they pick things up from the database next time).
    pub fn send_to(&self, user: Uuid, msg: &ServerMsg) {
        if let Some(list) = self.conns.lock().unwrap().get(&user) {
            for (_, tx) in list {
                let _ = tx.send(msg.clone());
            }
        }
    }

    pub fn is_online(&self, user: Uuid) -> bool {
        self.conns.lock().unwrap().contains_key(&user)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn delivers_to_all_of_a_users_connections_only() {
        let hub = Hub::default();
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let (_, _, mut a1) = hub.register(a);
        let (_, _, mut a2) = hub.register(a);
        let (_, _, mut b1) = hub.register(b);

        hub.send_to(a, &ServerMsg::Unauthorized);
        assert!(matches!(a1.recv().await, Some(ServerMsg::Unauthorized)));
        assert!(matches!(a2.recv().await, Some(ServerMsg::Unauthorized)));
        assert!(b1.try_recv().is_err());
    }

    #[tokio::test]
    async fn unregister_removes_the_connection() {
        let hub = Hub::default();
        let u = Uuid::new_v4();
        let (id, _, _rx) = hub.register(u);
        assert!(hub.is_online(u));
        hub.unregister(u, id);
        assert!(!hub.is_online(u));
    }
}
