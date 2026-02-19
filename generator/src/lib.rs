//! Data generator that produces random Message bytes on a background thread.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use protocol::{ESCAPE, Message, START_SEQUENCE};
use rand::{Rng, RngExt};
use tokio::sync::mpsc;

/// Configuration for the message generator.
#[derive(Clone, Debug)]
pub struct GeneratorConfig {
    /// Target messages per second.
    pub message_rate_per_sec: usize,
    /// Probability (0.0..=1.0) that an error (wrong checksum, invalid escape, or gap) is injected.
    pub error_probability: f64,
}

impl Default for GeneratorConfig {
    fn default() -> Self {
        Self {
            message_rate_per_sec: 1000,
            error_probability: 0.001,
        }
    }
}

/// Generator that runs a background thread and sends serialized message bytes on a channel.
pub struct Generator {
    stopped: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Generator {
    /// Start the generator; returns (generator, receiver). The background thread runs until the
    /// sender is dropped.
    pub fn new(config: GeneratorConfig) -> (Self, mpsc::Receiver<Vec<u8>>) {
        const PAUSE_INTERVAL: Duration = Duration::from_millis(100);

        let stopped = Arc::new(AtomicBool::new(false));
        let msgs_per_100_ms = config.message_rate_per_sec / 10;
        let (tx, rx) = mpsc::channel(msgs_per_100_ms);

        let handle = {
            let stopped = stopped.clone();
            thread::spawn(move || {
                let mut rng = rand::rng();
                let mut last_pause = Instant::now();
                let mut msgs_sent_since_last_pause = 0;

                loop {
                    if stopped.load(Ordering::Relaxed) {
                        break;
                    }

                    // We'll pause the thread every 1/10th of a second's worth of messages to
                    // roughly maintain the target message rate.
                    if msgs_sent_since_last_pause >= msgs_per_100_ms {
                        let duration_since_last_pause = last_pause.elapsed();
                        if duration_since_last_pause < PAUSE_INTERVAL {
                            thread::sleep(PAUSE_INTERVAL - duration_since_last_pause);
                        }
                        last_pause = Instant::now();
                        msgs_sent_since_last_pause = 0;
                    }

                    let inject_error = rng.random_bool(config.error_probability);
                    let bytes = if inject_error {
                        match rng.random_range(0..3) {
                            // Inject a checksum error, but modifying the checksum byte itself.
                            0 => {
                                let msg = random_message(&mut rng);
                                let mut bytes = msg.to_bytes();
                                if let Some(last) = bytes.last_mut() {
                                    // Increment the checksum byte.
                                    *last = last.wrapping_add(1);
                                }
                                bytes
                            }
                            // Inject an invalid escape sequence right after the start sequence.
                            1 => {
                                let mut bytes = Vec::from(START_SEQUENCE);
                                bytes.extend([ESCAPE, 0x01]);
                                let n = rng.random_range(0..=3);
                                bytes.extend((&mut rng).random_iter::<u8>().take(n));
                                bytes
                            }
                            // Inject a gap, by adding random bytes before a valid message.
                            _ => {
                                let gap_len = rng.random_range(1..=5);
                                let mut bytes =
                                    Vec::from_iter((&mut rng).random_iter::<u8>().take(gap_len));
                                random_message(&mut rng).write_bytes(&mut bytes).unwrap();
                                bytes
                            }
                        }
                    } else {
                        random_message(&mut rng).to_bytes()
                    };

                    if tx.blocking_send(bytes).is_err() {
                        break;
                    }
                    msgs_sent_since_last_pause += 1;
                }
            })
        };

        (
            Self {
                stopped,
                handle: Some(handle),
            },
            rx,
        )
    }

    /// Stop the generator.
    ///
    /// This drops the generator, which will gracefully stop the background thread.
    pub fn stop(self) {}
}

impl Drop for Generator {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn random_message<R: Rng + ?Sized>(rng: &mut R) -> Message {
    let address: u8 = rng.random();
    let destination: u8 = rng.random();
    let data_len = rng.random_range(0..=u8::MAX as usize);
    let data: Vec<u8> = rng.random_iter().take(data_len).collect();
    Message::builder()
        .address(address)
        .destination(destination)
        .data(data)
        .build()
        .unwrap()
}
