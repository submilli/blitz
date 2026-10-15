//! Synchronous image answers and unrelated messages held until a stage ends.
use std::collections::HashMap;
use std::sync::mpsc::Sender;

use crate::BaseDocument;
use crate::document::DocumentEvent;

pub(super) struct SynchronousAnswers {
    pending: Vec<Option<DocumentEvent>>,
    requests: HashMap<usize, Vec<usize>>,
    sender: Sender<DocumentEvent>,
}

impl SynchronousAnswers {
    pub(super) fn new(sender: Sender<DocumentEvent>) -> Self {
        Self {
            pending: Vec::new(),
            requests: HashMap::new(),
            sender,
        }
    }

    /// Apply only this request's answers. A later image can join an existing
    /// request whose answer an earlier step already set aside, so index those
    /// answers instead of repeatedly scanning all unrelated messages.
    pub(super) fn apply(&mut self, document: &mut BaseDocument, request: usize) {
        if let Some(indices) = self.requests.remove(&request) {
            for index in indices {
                if let Some(DocumentEvent::ResourceLoad(load)) = self.pending[index].take() {
                    document.load_resource(load);
                }
            }
        }
        let Some(receiver) = document.rx.take() else {
            return;
        };
        while let Ok(event) = receiver.try_recv() {
            match event {
                DocumentEvent::ResourceLoad(load) if load.request_id == request => {
                    document.load_resource(load);
                }
                other => self.hold(other),
            }
        }
        document.rx = Some(receiver);
    }

    fn hold(&mut self, event: DocumentEvent) {
        if let DocumentEvent::ResourceLoad(load) = &event {
            self.requests
                .entry(load.request_id)
                .or_default()
                .push(self.pending.len());
        }
        self.pending.push(Some(event));
    }
}

impl Drop for SynchronousAnswers {
    fn drop(&mut self) {
        // Restore unrelated answers once, in arrival order, even if the
        // caller abandons a snapshot. No document borrow or script is needed.
        for event in self.pending.drain(..).flatten() {
            let _ = self.sender.send(event);
        }
    }
}
