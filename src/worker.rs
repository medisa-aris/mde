//! Parses Markdown into preview blocks away from the UI thread.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Instant;

use crate::preview::blocks::{Block, split};

struct Request {
    doc_id: u64,
    revision: u64,
    text: String,
    sent: Instant,
}

pub struct Parsed {
    pub doc_id: u64,
    pub revision: u64,
    pub blocks: Arc<[Block]>,
    /// When the matching request was made, for timing.
    pub sent: Instant,
}

pub struct Worker {
    requests: Sender<Request>,
    results: Receiver<Parsed>,
}

impl Worker {
    /// `wake` is called after each result so the UI repaints.
    pub fn spawn(wake: impl Fn() + Send + 'static) -> Self {
        let (requests, request_rx) = channel::<Request>();
        let (result_tx, results) = channel();
        std::thread::Builder::new()
            .name("markdown-parser".into())
            .spawn(move || {
                while let Ok(first) = request_rx.recv() {
                    for request in newest_per_document(first, request_rx.try_iter()) {
                        let parsed = Parsed {
                            doc_id: request.doc_id,
                            revision: request.revision,
                            blocks: split(&request.text).into(),
                            sent: request.sent,
                        };
                        if result_tx.send(parsed).is_err() {
                            return;
                        }
                        wake();
                    }
                }
            })
            .expect("failed to start the parser thread");
        Self { requests, results }
    }

    pub fn request(&self, doc_id: u64, revision: u64, text: &str) {
        // The thread only stops when this struct is dropped.
        let _ = self.requests.send(Request {
            doc_id,
            revision,
            text: text.to_owned(),
            sent: Instant::now(),
        });
    }

    pub fn results(&self) -> impl Iterator<Item = Parsed> + '_ {
        self.results.try_iter()
    }
}

/// Drops requests that were superseded while the worker was busy.
fn newest_per_document(first: Request, rest: impl Iterator<Item = Request>) -> Vec<Request> {
    let mut newest = vec![first];
    for request in rest {
        match newest.iter_mut().find(|r| r.doc_id == request.doc_id) {
            Some(slot) => *slot = request,
            None => newest.push(request),
        }
    }
    newest
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(doc_id: u64, revision: u64) -> Request {
        Request {
            doc_id,
            revision,
            text: String::new(),
            sent: Instant::now(),
        }
    }

    #[test]
    fn stale_requests_are_dropped() {
        let rest = vec![request(1, 2), request(2, 1), request(1, 3)];
        let kept: Vec<_> = newest_per_document(request(1, 1), rest.into_iter())
            .iter()
            .map(|r| (r.doc_id, r.revision))
            .collect();
        assert_eq!(kept, vec![(1, 3), (2, 1)]);
    }
}
