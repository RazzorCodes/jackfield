use crate::components::endpoint::*;
use crate::components::message::*;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

struct Channel {
    sender: Sender<Box<dyn Message>>,
    receiver: Receiver<Box<dyn Message>>,
}

impl Channel {
    fn new() -> Self {
        let (sender, receiver) = mpsc::channel();
        Channel { sender, receiver }
    }

    fn sender(&self) -> Sender<Box<dyn Message>> {
        self.sender.clone()
    }

    fn drain(&self) -> Vec<Box<dyn Message>> {
        self.receiver.try_iter().collect()
    }

    fn is_empty(&self) -> bool {
        let result = self.receiver.try_recv();
        let empty = matches!(result, Err(TryRecvError::Empty));
        if !empty {
            self.requeue(result.unwrap());
        }

        return empty;
    }

    fn requeue(&self, message: Box<dyn Message>) {
        let _ = self.sender.send(message);
    }
}

struct Registry {
    consumers: Vec<Box<dyn Consumer>>,
}

impl Registry {
    fn new() -> Self {
        Registry {
            consumers: Vec::new(),
        }
    }

    fn register(&mut self, consumer: Box<dyn Consumer>) {
        self.consumers.push(consumer);
    }

    fn route(&mut self, message: Box<dyn Message>) -> Option<Box<dyn Message>> {
        if let Some(idx) = self.consumers.iter().position(|c| c.validate(&*message)) {
            self.consumers[idx].consume(message);
            None // consumed
        } else {
            Some(message) // no consumer matched → return it
        }
    }
}

pub struct Bus {
    channel: Channel,
    registry: Registry,
}

impl Bus {
    pub fn new() -> Self {
        Bus {
            channel: Channel::new(),
            registry: Registry::new(),
        }
    }

    pub fn register_consumer(&mut self, consumer: Box<dyn Consumer>) {
        self.registry.register(consumer);
    }

    pub fn register_producer(&mut self, producer: &mut dyn Producer) {
        producer.attach(self.channel.sender());
    }

    pub fn done(&self) -> bool {
        self.channel.is_empty()
    }

    pub fn route_sync(&mut self) {
        for message in self.channel.drain() {
            if let Some(unhandled) = self.registry.route(message) {
                self.channel.requeue(unhandled);
            }
        }
    }
}

impl Default for Bus {
    fn default() -> Self {
        Self::new()
    }
}
