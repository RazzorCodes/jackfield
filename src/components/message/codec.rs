// Protobuf codec: generated BusableItem type + From conversions to/from Box<dyn Message>.
pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/jackfield.rs"));
}

use crate::components::message::{BaseMessage, Message};
use uuid::Uuid;

impl From<Box<dyn Message>> for proto::BusableItem {
    fn from(msg: Box<dyn Message>) -> Self {
        proto::BusableItem {
            uuid: msg.get_uuid().as_bytes().to_vec(),
            labels: msg.get_labels().to_vec(),
            data: msg.get_bytes().to_vec(),
        }
    }
}

impl From<proto::BusableItem> for Box<dyn Message> {
    fn from(proto: proto::BusableItem) -> Self {
        let uuid = Uuid::from_slice(&proto.uuid).unwrap_or_default();
        Box::new(BaseMessage::new(
            Some(uuid),
            Some(proto.labels),
            Some(proto.data),
        ))
    }
}
