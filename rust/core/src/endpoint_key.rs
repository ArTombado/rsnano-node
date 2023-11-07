use crate::utils::{Deserialize, FixedSizeSerialize, MutStreamAdapter, Serialize, Stream};

#[derive(Default, PartialEq, Eq, Debug, Clone)]
pub struct EndpointKey {
    /// The ipv6 address in network byte order
    address: [u8; 16],

    /// The port in host byte order
    port: u16,
}

impl EndpointKey {
    /// address in network byte order, port in host byte order
    pub fn new(address: [u8; 16], port: u16) -> Self {
        Self { address, port }
    }

    pub fn to_bytes(&self) -> [u8; 18] {
        let mut buffer = [0; 18];
        let mut stream = MutStreamAdapter::new(&mut buffer);
        self.serialize_safe(&mut stream);
        buffer
    }

    pub fn create_test_instance() -> Self {
        EndpointKey::new([1; 16], 123)
    }
}

impl Serialize for EndpointKey {
    fn serialize(&self, stream: &mut dyn Stream) -> anyhow::Result<()> {
        stream.write_bytes(&self.address)?;
        stream.write_bytes(&self.port.to_be_bytes())
    }

    fn serialize_safe(&self, stream: &mut MutStreamAdapter) {
        stream.write_bytes_safe(&self.address);
        stream.write_bytes_safe(&self.port.to_be_bytes());
    }
}

impl FixedSizeSerialize for EndpointKey {
    fn serialized_size() -> usize {
        18
    }
}

impl Deserialize for EndpointKey {
    type Target = Self;
    fn deserialize(stream: &mut dyn Stream) -> anyhow::Result<EndpointKey> {
        let mut result = EndpointKey {
            address: Default::default(),
            port: 0,
        };
        stream.read_bytes(&mut result.address, 16)?;
        let mut buffer = [0; 2];
        stream.read_bytes(&mut buffer, 2)?;
        result.port = u16::from_be_bytes(buffer);
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use crate::utils::{MemoryStream, StreamAdapter};
    use std::{net::Ipv6Addr, str::FromStr};

    use super::*;

    #[test]
    fn deserialize() {
        let ip = Ipv6Addr::from_str("::ffff:127.0.0.1").unwrap();
        let key = EndpointKey::new(ip.octets(), 123);
        let mut buf = [0; 18];
        let mut stream = MutStreamAdapter::new(&mut buf);
        key.serialize_safe(&mut stream);
        let deserialized = EndpointKey::deserialize(&mut StreamAdapter::new(&buf)).unwrap();
        assert_eq!(deserialized, key);
    }

    #[test]
    fn byte_order() {
        let ip = Ipv6Addr::from_str("::ffff:127.0.0.1").unwrap();
        let key = EndpointKey::new(ip.octets(), 100);
        let mut stream = MemoryStream::new();
        key.serialize(&mut stream).unwrap();
        let bytes = stream.as_bytes();
        assert_eq!(bytes.len(), 18);
        assert_eq!(bytes[10], 0xFF);
        assert_eq!(bytes[11], 0xFF);
        assert_eq!(bytes[12], 127);
        assert_eq!(bytes[16], 0);
        assert_eq!(bytes[17], 100);
    }
}
