use uuid::Uuid;

pub trait Message: Send + Sync {
    fn get_uuid(&self) -> Uuid;
    fn aquire_uuid(&mut self);
    fn get_labels(&self) -> &[String];
    fn get_bytes(&self) -> &[u8];
}

pub struct BaseMessage {
    pub uuid: Uuid,
    pub labels: Vec<String>,
    pub data: Vec<u8>,
}

impl BaseMessage {
    pub fn new(uuid: Option<Uuid>, labels: Option<Vec<String>>, data: Option<Vec<u8>>) -> Self {
        Self {
            uuid: uuid.unwrap_or(Uuid::nil()),
            labels: labels.unwrap_or_else(|| vec![]),
            data: data.unwrap_or_else(|| vec![]),
        }
    }
}

impl Message for BaseMessage {
    fn get_uuid(&self) -> Uuid {
        self.uuid
    }
    fn aquire_uuid(&mut self) {
        self.uuid = Uuid::new_v4();
    }
    fn get_labels(&self) -> &[String] {
        &self.labels[..]
    }
    fn get_bytes(&self) -> &[u8] {
        &self.data[..]
    }
}

// ---

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_creation() {
        let message = BaseMessage::new(None, None, None);

        assert!(true)
    }
}
