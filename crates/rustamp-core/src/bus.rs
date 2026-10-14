//! In-process message bus decoupling UI, audio, platform and plugin bridge.
//!
//! * Commands flow many-to-one: any number of cloneable [`CommandSender`]s,
//!   one [`CommandReceiver`] owned by the app/state owner.
//! * Events flow one-to-many: the owner calls [`EventBus::publish`], every
//!   [`EventBus::subscribe`] receiver gets its own copy. Subscribers that
//!   drop their receiver are pruned automatically. Channels are unbounded,
//!   so subscribers must drain regularly; do not publish from the audio thread.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, TryRecvError};

use crate::command::Command;
use crate::error::{Error, Result};
use crate::event::Event;

/// Cloneable handle for sending [`Command`]s.
#[derive(Debug, Clone)]
pub struct CommandSender(Sender<Command>);

/// Receiving end of the command channel.
#[derive(Debug)]
pub struct CommandReceiver(Receiver<Command>);

/// Create a connected command sender/receiver pair.
pub fn command_channel() -> (CommandSender, CommandReceiver) {
    let (tx, rx) = crossbeam_channel::unbounded();
    (CommandSender(tx), CommandReceiver(rx))
}

impl CommandSender {
    /// Send a command; fails with [`Error::Disconnected`] if the receiver is gone.
    pub fn send(&self, cmd: Command) -> Result<()> {
        self.0.send(cmd).map_err(|_| Error::Disconnected)
    }
}

impl CommandReceiver {
    /// Block until a command arrives; `Err(Disconnected)` when all senders are gone.
    pub fn recv(&self) -> Result<Command> {
        self.0.recv().map_err(|_| Error::Disconnected)
    }

    /// Non-blocking receive; `Ok(None)` if empty.
    pub fn try_recv(&self) -> Result<Option<Command>> {
        match self.0.try_recv() {
            Ok(c) => Ok(Some(c)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(Error::Disconnected),
        }
    }

    /// Receive with a timeout; `Ok(None)` on timeout.
    pub fn recv_timeout(&self, timeout: Duration) -> Result<Option<Command>> {
        match self.0.recv_timeout(timeout) {
            Ok(c) => Ok(Some(c)),
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => Ok(None),
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => Err(Error::Disconnected),
        }
    }

    /// Drain all currently queued commands.
    pub fn try_iter(&self) -> impl Iterator<Item = Command> + '_ {
        self.0.try_iter()
    }
}

/// Broadcast hub for [`Event`]s. Cheap to clone; clones share subscribers.
#[derive(Debug, Clone, Default)]
pub struct EventBus {
    subscribers: Arc<Mutex<Vec<Sender<Event>>>>,
}

/// A subscription to an [`EventBus`].
#[derive(Debug)]
pub struct EventReceiver(Receiver<Event>);

impl EventBus {
    /// New bus without subscribers.
    pub fn new() -> Self {
        Self::default()
    }

    /// Subscribe; only events published after this call are received.
    pub fn subscribe(&self) -> EventReceiver {
        let (tx, rx) = crossbeam_channel::unbounded();
        self.lock().push(tx);
        EventReceiver(rx)
    }

    /// Publish an event to all live subscribers.
    pub fn publish(&self, event: Event) {
        self.lock().retain(|tx| tx.send(event.clone()).is_ok());
    }

    /// Number of live subscribers (as of the last publish).
    pub fn subscriber_count(&self) -> usize {
        self.lock().len()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Sender<Event>>> {
        self.subscribers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

impl EventReceiver {
    /// Block for the next event; `Err(Disconnected)` if the bus is gone.
    pub fn recv(&self) -> Result<Event> {
        self.0.recv().map_err(|_| Error::Disconnected)
    }

    /// Non-blocking receive; `Ok(None)` if empty.
    pub fn try_recv(&self) -> Result<Option<Event>> {
        match self.0.try_recv() {
            Ok(e) => Ok(Some(e)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(Error::Disconnected),
        }
    }

    /// Receive with a timeout; `Ok(None)` on timeout.
    pub fn recv_timeout(&self, timeout: Duration) -> Result<Option<Event>> {
        match self.0.recv_timeout(timeout) {
            Ok(e) => Ok(Some(e)),
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => Ok(None),
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => Err(Error::Disconnected),
        }
    }

    /// Drain all currently queued events.
    pub fn try_iter(&self) -> impl Iterator<Item = Event> + '_ {
        self.0.try_iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::TransportState;

    #[test]
    fn commands_flow() {
        let (tx, rx) = command_channel();
        tx.clone().send(Command::Play).unwrap();
        tx.send(Command::Stop).unwrap();
        assert_eq!(
            rx.try_iter().collect::<Vec<_>>(),
            [Command::Play, Command::Stop]
        );
        assert_eq!(rx.try_recv().unwrap(), None);
        drop(tx);
        assert!(matches!(rx.try_recv(), Err(Error::Disconnected)));
    }

    #[test]
    fn events_broadcast_and_prune() {
        let bus = EventBus::new();
        let a = bus.subscribe();
        let b = bus.subscribe();
        bus.publish(Event::StateChanged(TransportState::Playing));
        assert_eq!(
            a.try_recv().unwrap(),
            Some(Event::StateChanged(TransportState::Playing))
        );
        assert_eq!(
            b.try_recv().unwrap(),
            Some(Event::StateChanged(TransportState::Playing))
        );
        drop(b);
        bus.publish(Event::Quit);
        assert_eq!(bus.subscriber_count(), 1);
        assert_eq!(a.try_recv().unwrap(), Some(Event::Quit));
    }
}
