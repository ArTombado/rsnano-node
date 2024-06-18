use crate::stats::{DetailType, StatType, Stats};

use super::{ChannelEnum, FairQueue, Origin};
use rsnano_core::{utils::ContainerInfoComponent, NoValue};
use rsnano_messages::DeserializedMessage;
use std::{
    collections::VecDeque,
    sync::{Arc, Condvar, Mutex},
};

pub struct InboundMessageQueue {
    state: Mutex<State>,
    condition: Condvar,
    stats: Arc<Stats>,
}

impl InboundMessageQueue {
    pub fn new(max_queue: usize, stats: Arc<Stats>) -> Self {
        Self {
            state: Mutex::new(State {
                queue: FairQueue::new(Box::new(move |_| max_queue), Box::new(|_| 1)),
                stopped: false,
            }),
            condition: Condvar::new(),
            stats,
        }
    }

    pub fn put(&self, message: DeserializedMessage, channel: Arc<ChannelEnum>) -> bool {
        let message_type = message.message.message_type();
        let added = self
            .state
            .lock()
            .unwrap()
            .queue
            .push((message, channel.clone()), Origin::new(NoValue {}, channel));

        if added {
            self.stats
                .inc(StatType::MessageProcessor, DetailType::Process);
            self.stats
                .inc(StatType::MessageProcessorType, message_type.into());

            self.condition.notify_all();
        } else {
            self.stats
                .inc(StatType::MessageProcessor, DetailType::Overfill);
            self.stats
                .inc(StatType::MessageProcessorOverfill, message_type.into());
        }

        added
    }

    pub fn next_batch(
        &self,
        max_batch_size: usize,
    ) -> VecDeque<((DeserializedMessage, Arc<ChannelEnum>), Origin<NoValue>)> {
        self.state.lock().unwrap().queue.next_batch(max_batch_size)
    }

    pub fn wait_for_messages(&self) {
        let state = self.state.lock().unwrap();
        if !state.queue.is_empty() {
            return;
        }
        drop(
            self.condition
                .wait_while(state, |s| !s.stopped && s.queue.is_empty()),
        )
    }

    pub fn size(&self) -> usize {
        self.state.lock().unwrap().queue.len()
    }

    /// Stop container and notify waiting threads
    pub fn stop(&self) {
        {
            let mut lock = self.state.lock().unwrap();
            lock.stopped = true;
        }
        self.condition.notify_all();
    }

    pub fn collect_container_info(&self, name: impl Into<String>) -> ContainerInfoComponent {
        ContainerInfoComponent::Composite(
            name.into(),
            vec![self
                .state
                .lock()
                .unwrap()
                .queue
                .collect_container_info("queue")],
        )
    }
}

impl Default for InboundMessageQueue {
    fn default() -> Self {
        Self::new(64, Arc::new(Stats::default()))
    }
}

struct State {
    queue: FairQueue<(DeserializedMessage, Arc<ChannelEnum>), NoValue>,
    stopped: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsnano_messages::Message;

    #[test]
    fn put_and_get_one_message() {
        let manager = InboundMessageQueue::new(1, Arc::new(Stats::default()));
        assert_eq!(manager.size(), 0);
        manager.put(
            DeserializedMessage::new(Message::BulkPush, Default::default()),
            Arc::new(ChannelEnum::new_null()),
        );
        assert_eq!(manager.size(), 1);
        assert_eq!(manager.next_batch(1000).len(), 1);
        assert_eq!(manager.size(), 0);
    }
}
