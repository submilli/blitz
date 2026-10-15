//! Initiated data sources survive element removal, but decode only at a safe
//! stable-state checkpoint. Both count and retained URL bytes are bounded.
use crate::NodeId;
use crate::image_request::ImageKey;

const MAX_DATA_UPDATES: usize = 1024;
const MAX_DATA_URL_BYTES: usize = 16 * 1024 * 1024;

pub(crate) struct DataImageUpdate {
    pub(super) node: NodeId,
    pub(super) key: ImageKey,
}

#[derive(Default)]
pub(super) struct DataImageUpdates {
    queued: Vec<DataImageUpdate>,
    url_bytes: usize,
}

impl DataImageUpdates {
    pub(super) fn is_empty(&self) -> bool {
        self.queued.is_empty()
    }

    pub(super) fn push(&mut self, node: NodeId, key: ImageKey) -> bool {
        if self.queued.len() >= MAX_DATA_UPDATES
            || key.url.len() > MAX_DATA_URL_BYTES.saturating_sub(self.url_bytes)
        {
            return false;
        }
        self.url_bytes += key.url.len();
        self.queued.push(DataImageUpdate { node, key });
        true
    }

    pub(super) fn take(&mut self) -> Vec<DataImageUpdate> {
        self.url_bytes = 0;
        std::mem::take(&mut self.queued)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_urls_are_bounded_independently_of_job_count() {
        let mut queue = DataImageUpdates::default();
        let node = NodeId::from_u64(0);
        let key = ImageKey::no_cors("x".repeat(MAX_DATA_URL_BYTES));
        assert!(queue.push(node, key));
        assert!(!queue.push(node, ImageKey::no_cors("x")));
        assert_eq!(queue.take().len(), 1);
        assert!(queue.push(node, ImageKey::no_cors("x")));
    }
}
