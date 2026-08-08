//! The audio command transport: a bounded, lossless queue from the thread
//! running the game to the audio thread.
//!
//! **Why it is bounded, and why bounding it is not the input queue's problem
//! again.** Section 7.3 forbids unbounded queue growth under saturation. This
//! queue used to be `tokio::sync::mpsc::unbounded_channel`, drained at most
//! [`AUDIO_COMMANDS_PER_DRAIN`] commands per audio-thread iteration with the rest
//! deferred — an unbounded queue behind a capped drain, which is that growth
//! shape exactly, and the drain's own reason for existing names the producer that
//! reaches it: a game firing rapid bursts of automation or sound effects.
//!
//! The input queue answers saturation by coalescing replaceable state and letting
//! a terminal transition supersede it. Nothing here is replaceable. `AudioCmd`
//! carries ids allocated on the JavaScript side with fire-and-forget creates, so
//! ordering *is* the protocol: drop one command and a later one addresses a node
//! that was never created. Refusing to JavaScript is no better, because the Web
//! Audio API has no error for it.
//!
//! So this takes the render command path's answer instead — a bounded channel
//! whose send waits — which is backpressure without loss.
//!
//! **Waiting is bounded, and that rests on two facts about the consumer rather
//! than on hope.** The audio thread never blocks: its device write is a push onto
//! a lock-free ring that reports how much it took. And every send notifies the
//! thread's [`ThreadWakeup`](crate::channel::ThreadWakeup), whose signal is
//! latched, so a notification cannot be lost against a consumer about to sleep.
//! A producer therefore waits at most the few iterations it takes to drain a full
//! queue, which is what [`AUDIO_COMMAND_CAPACITY`]'s derivation from
//! [`AUDIO_COMMANDS_PER_DRAIN`] is for.

use crate::protocol::audio_cmd::AudioCmd;

/// How many commands the audio thread takes from the queue in one iteration.
///
/// The cap exists so a burst cannot starve mixing: whatever is left waits for the
/// next iteration, which a send's own wakeup brings forward immediately.
pub const AUDIO_COMMANDS_PER_DRAIN: usize = 256;

/// How many commands the queue holds before a send waits.
///
/// Derived rather than chosen: a full queue must empty within a small fixed
/// number of consumer iterations, because that count *is* the bound on how long
/// a saturating producer waits. Four drains is the number.
pub const AUDIO_COMMAND_CAPACITY: usize = 4 * AUDIO_COMMANDS_PER_DRAIN;

// A capacity below one drain would leave a producer waiting a whole iteration per
// command, which is a bound in name only.
const _: () = assert!(AUDIO_COMMAND_CAPACITY >= AUDIO_COMMANDS_PER_DRAIN);

pub type AudioCommandSender = crossbeam_channel::Sender<AudioCmd>;
pub type AudioCommandReceiver = crossbeam_channel::Receiver<AudioCmd>;

/// The transport the audio thread is built on.
pub fn channel() -> (AudioCommandSender, AudioCommandReceiver) {
    crossbeam_channel::bounded(AUDIO_COMMAND_CAPACITY)
}

/// A sender with no consumer and no queue, for a build that has no audio
/// subsystem to send to.
///
/// **A queue nobody drains is the thing this must not be.** The profile without
/// `api-media` used to hold a live receiver it never read, so a send would have
/// queued for the life of the session; that was harmless only because the audio
/// ops are compiled out of that profile and nothing could reach it. Behind a
/// bounded channel the same shape is worse than a leak — the first producer to
/// fill it waits forever. With no receiver at all, a send fails at once and
/// hands the command back, and the reason it is safe stops being a fact about
/// which ops happen to be registered.
pub fn disconnected() -> AudioCommandSender {
    // Zero capacity: there is no consumer, so there is no queue worth holding
    // slots for either.
    let (tx, rx) = crossbeam_channel::bounded(0);
    drop(rx);
    tx
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::ThreadWakeup;
    use crate::op_state::AudioSender;
    use migo_alloc_probe::{Burst, assert_no_steady_state_allocation};
    use std::sync::mpsc;
    use std::time::Duration;

    /// Generous: correct code returns in microseconds, and only code that parks
    /// forever reaches this.
    const LIVENESS_DEADLINE: Duration = Duration::from_secs(10);

    fn command(ctx_id: u32) -> AudioCmd {
        AudioCmd::CreateContext {
            ctx_id,
            sample_rate: None,
        }
    }

    /// Section 7.3's bounded-hot-paths requirement, stated about the transport
    /// itself. An unbounded channel reports no capacity at all, which is the
    /// difference this asserts.
    #[test]
    fn the_transport_is_bounded() {
        let (tx, _rx) = channel();

        assert_eq!(
            tx.capacity(),
            Some(AUDIO_COMMAND_CAPACITY),
            "the audio command transport is unbounded, so a producer faster than \
             the drain grows it without limit"
        );
    }

    /// Boundedness as the producer meets it: past capacity the queue refuses and
    /// hands the command back rather than taking it or dropping it. The same
    /// policy the input queue and the deferred upload queue use.
    #[test]
    fn past_capacity_the_queue_hands_the_command_back() {
        let (tx, _rx) = channel();
        for ctx_id in 0..AUDIO_COMMAND_CAPACITY as u32 {
            tx.try_send(command(ctx_id))
                .expect("capacity must accept its own count");
        }

        let refused = tx.try_send(command(9999));

        assert!(
            matches!(
                refused,
                Err(crossbeam_channel::TrySendError::Full(
                    AudioCmd::CreateContext { ctx_id: 9999, .. }
                ))
            ),
            "a full queue took the command anyway, or lost it: {refused:?}"
        );
    }

    /// **The hazard bounding this queue creates, and the reason the notification
    /// happens before the wait rather than after the send.** The audio thread
    /// sleeps indefinitely once content has been silent, and it is a send's own
    /// wakeup that brings it back. A send that parked on a full queue and only
    /// notified afterwards would wait for a consumer that is waiting for it: the
    /// slot is freed by a drain, the drain needs the wakeup, and the wakeup comes
    /// after the send returns.
    ///
    /// The queue is filled through the raw sender, which does not notify, so the
    /// consumer really is asleep when the measured send arrives — filling it
    /// through `AudioSender` would wake the consumer on the first command and the
    /// queue would never reach capacity.
    ///
    /// Observed with a deadline rather than a bare `join`, because the failure
    /// mode is a thread that never returns: a hung suite is not a test result.
    #[test]
    fn a_send_into_a_full_queue_wakes_the_sleeping_consumer_that_frees_it() {
        let (tx, rx) = channel();
        let wakeup = ThreadWakeup::new();
        for ctx_id in 0..AUDIO_COMMAND_CAPACITY as u32 {
            tx.try_send(command(ctx_id)).expect("fixture must fill it");
        }
        assert_eq!(rx.len(), AUDIO_COMMAND_CAPACITY);

        let consumer_wakeup = wakeup.clone();
        let consumer = std::thread::spawn(move || {
            // Exactly what the audio thread does: sleep until told, then take one
            // drain's worth.
            consumer_wakeup.wait();
            for _ in 0..AUDIO_COMMANDS_PER_DRAIN {
                if rx.try_recv().is_err() {
                    break;
                }
            }
            rx
        });

        let (returned_tx, returned_rx) = mpsc::channel();
        let sender = AudioSender::new(tx, wakeup);
        let producer = std::thread::spawn(move || {
            let outcome = sender.send(command(9999));
            let _ = returned_tx.send(outcome.is_ok());
        });

        let accepted = returned_rx.recv_timeout(LIVENESS_DEADLINE).expect(
            "the send never returned: it parked on the full queue without waking the \
             consumer whose drain is the only thing that can free a slot",
        );
        assert!(accepted, "the send failed rather than waiting for a slot");
        consumer.join().expect("consumer thread panicked");
        producer.join().expect("producer thread panicked");
    }

    /// A closed transport must hand the command back at once. Waiting for a slot
    /// that no consumer will ever free is the one way a bounded queue can be
    /// worse than an unbounded one.
    #[test]
    fn a_disconnected_transport_returns_the_command_instead_of_waiting() {
        let (tx, rx) = channel();
        drop(rx);

        let outcome = AudioSender::new(tx, ThreadWakeup::new()).send(command(1));

        assert!(
            matches!(
                outcome,
                Err(crossbeam_channel::SendError(AudioCmd::CreateContext {
                    ctx_id: 1,
                    ..
                }))
            ),
            "a send with no consumer did not return its command: {outcome:?}"
        );
    }

    /// The profile with no audio subsystem. Every send fails immediately, so
    /// nothing accumulates and nothing waits — the property the previous
    /// never-drained receiver had only by virtue of the ops being compiled out.
    #[test]
    fn a_disconnected_sender_holds_no_queue_and_refuses_every_send() {
        let sender = AudioSender::new(disconnected(), ThreadWakeup::new());

        for ctx_id in 0..AUDIO_COMMAND_CAPACITY as u32 + 1 {
            assert!(
                sender.send(command(ctx_id)).is_err(),
                "a build with no audio consumer queued command {ctx_id}"
            );
        }
    }

    /// Section 7.3's zero-allocation requirement, on a per-event path: one
    /// JavaScript audio call is one send. The unbounded channel this replaced
    /// bought a block from the heap every thirty-two messages, forever, on the
    /// thread running the game.
    ///
    /// One iteration is a send and the drain that matches it, so the queue neither
    /// fills nor makes the send wait — a burst that filled it would be measuring
    /// the wait instead.
    #[test]
    fn a_steady_state_audio_command_send_never_reaches_the_heap() {
        const WARMUP: usize = 4;
        const MEASURED: usize = 64;

        let (tx, rx) = channel();
        let sender = AudioSender::new(tx, ThreadWakeup::new());

        assert_no_steady_state_allocation(
            Burst {
                path: "audio transport: enqueue one command and take it",
                warmup: WARMUP,
                measured: MEASURED,
            },
            |iteration| {
                sender
                    .send(command(iteration as u32))
                    .expect("the consumer is this closure, so a slot is always free");
                std::hint::black_box(rx.try_recv().is_ok())
            },
        );
    }
}
