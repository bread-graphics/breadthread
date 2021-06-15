// MIT/Apache2 License

use crate::DirectiveAdaptor;
use flume::{Sender, TryRecvError};
use orphan_crippler::Sender as OcSender;
use std::thread;

/// Things we need to tell a directive thread to do.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum DirectiveThreadMessage {
    // NOTE: we'll probably eventually need more
    /// Tell the directive thread to stop.
    Stop,
}

/// Launch the directive thread.
#[inline]
pub(crate) fn launch_directive_thread<
    Dir: Send + 'static,
    Da: DirectiveAdaptor<Dir> + Send + 'static,
>(
    mut adaptor: Da,
) -> (
    Sender<DirectiveThreadMessage>,
    Sender<Option<OcSender<Dir>>>,
) {
    let (message_sender, message_receiver) = flume::unbounded::<DirectiveThreadMessage>();
    let (directive_sender, directive_receiver) = flume::unbounded::<Option<OcSender<Dir>>>();
    thread::Builder::new()
        .name("directive_thread".to_string())
        .spawn(move || loop {
            match message_receiver.try_recv() {
                // if we don't have anything, move onto the directives
                Err(TryRecvError::Empty) => (),
                // if we're disconnected or we're ordered to stop, stop
                Err(TryRecvError::Disconnected) | Ok(DirectiveThreadMessage::Stop) => break,
            }

            match directive_receiver.recv() {
                Err(_) => break,
                Ok(None) => (),
                Ok(Some(directive)) => adaptor.send(directive),
            }
        })
        .expect("Unable to spawn directive thread");
    (message_sender, directive_sender)
}
