pub mod broadcast_tracker;

use alloc::boxed::Box;
use async_trait::async_trait;

#[async_trait]
pub(crate) trait Tracker {
    fn new(size: usize) -> Self;
    fn new_receiver(&self) -> usize;
    fn move_receiver(&self, from: usize, to: usize);
    fn remove_receiver(&self, cell: usize);
    fn slowest(&self) -> usize;
    fn wait_for_tail(&self, expected_tail: usize) -> usize;
    async fn wait_for_tail_async(&self, expected_tail: usize) -> usize;
}
