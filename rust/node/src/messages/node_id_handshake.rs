use super::MessageVariant;
use crate::transport::Cookie;
use anyhow::Result;
use bitvec::prelude::BitArray;
use rand::{thread_rng, Rng};
use rsnano_core::{
    sign_message,
    utils::{BufferWriter, Deserialize, FixedSizeSerialize, MemoryStream, Serialize, Stream},
    validate_message, write_hex_bytes, Account, BlockHash, KeyPair, PublicKey, Signature,
};
use std::fmt::Display;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct NodeIdHandshakeQuery {
    pub cookie: [u8; 32],
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct NodeIdHandshakeResponse {
    pub node_id: Account,
    pub signature: Signature,
    pub v2: Option<V2Payload>,
}

impl NodeIdHandshakeResponse {
    pub fn new_v1(cookie: &Cookie, node_id: &KeyPair) -> Self {
        let mut response = Self {
            node_id: node_id.public_key(),
            signature: Signature::default(),
            v2: None,
        };
        response.sign(cookie, node_id);
        response
    }

    pub fn new_v2(cookie: &Cookie, node_id: &KeyPair, genesis: BlockHash) -> Self {
        let mut salt = [0; 32];
        thread_rng().fill(&mut salt);

        let mut response = Self {
            node_id: node_id.public_key(),
            signature: Signature::default(),
            v2: Some(V2Payload { salt, genesis }),
        };
        response.sign(cookie, node_id);
        response
    }

    pub fn sign(&mut self, cookie: &Cookie, key: &KeyPair) {
        debug_assert!(key.public_key() == self.node_id);
        let data = self.data_to_sign(cookie);
        self.signature = sign_message(&key.private_key(), &key.public_key(), &data);
        debug_assert!(self.validate(cookie).is_ok());
    }

    pub fn validate(&self, cookie: &Cookie) -> anyhow::Result<()> {
        let data = self.data_to_sign(cookie);
        validate_message(&self.node_id, &data, &self.signature)
    }

    fn data_to_sign(&self, cookie: &Cookie) -> Vec<u8> {
        let mut stream = MemoryStream::new();
        match &self.v2 {
            Some(v2) => {
                stream.write_bytes(cookie).unwrap();
                stream.write_bytes(&v2.salt).unwrap();
                v2.genesis.serialize(&mut stream).unwrap();
            }
            None => stream.write_bytes(cookie).unwrap(),
        }
        stream.to_vec()
    }

    pub fn deserialize(stream: &mut dyn Stream, extensions: BitArray<u16>) -> Result<Self> {
        if NodeIdHandshake::has_v2_flag(extensions) {
            let node_id = Account::deserialize(stream)?;
            let mut salt = [0u8; 32];
            stream.read_bytes(&mut salt, 32)?;
            let genesis = BlockHash::deserialize(stream)?;
            let signature = Signature::deserialize(stream)?;
            Ok(Self {
                node_id,
                signature,
                v2: Some(V2Payload { salt, genesis }),
            })
        } else {
            let node_id = Account::deserialize(stream)?;
            let signature = Signature::deserialize(stream)?;
            Ok(Self {
                node_id,
                signature,
                v2: None,
            })
        }
    }

    pub fn serialized_size(extensions: BitArray<u16>) -> usize {
        if NodeIdHandshake::has_v2_flag(extensions) {
            Account::serialized_size()
                + 32 // salt
                + BlockHash::serialized_size()
                + Signature::serialized_size()
        } else {
            Account::serialized_size() + Signature::serialized_size()
        }
    }
}

impl Serialize for NodeIdHandshakeResponse {
    fn serialize(&self, stream: &mut dyn Stream) -> Result<()> {
        match &self.v2 {
            Some(v2) => {
                self.node_id.serialize(stream)?;
                stream.write_bytes(&v2.salt)?;
                v2.genesis.serialize(stream)?;
                self.signature.serialize(stream)?;
            }
            None => {
                self.node_id.serialize(stream)?;
                self.signature.serialize(stream)?;
            }
        }
        Ok(())
    }

    fn serialize_safe(&self, stream: &mut dyn BufferWriter) {
        match &self.v2 {
            Some(v2) => {
                self.node_id.serialize_safe(stream);
                stream.write_bytes_safe(&v2.salt);
                v2.genesis.serialize_safe(stream);
                self.signature.serialize_safe(stream);
            }
            None => {
                self.node_id.serialize_safe(stream);
                self.signature.serialize_safe(stream);
            }
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct V2Payload {
    pub salt: [u8; 32],
    pub genesis: BlockHash,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct NodeIdHandshake {
    pub query: Option<NodeIdHandshakeQuery>,
    pub response: Option<NodeIdHandshakeResponse>,
    pub is_v2: bool,
}

impl NodeIdHandshake {
    pub const QUERY_FLAG: usize = 0;
    pub const RESPONSE_FLAG: usize = 1;
    pub const V2_FLAG: usize = 2;

    pub fn is_query(extensions: BitArray<u16>) -> bool {
        extensions[NodeIdHandshake::QUERY_FLAG]
    }

    pub fn is_response(extensions: BitArray<u16>) -> bool {
        extensions[NodeIdHandshake::RESPONSE_FLAG]
    }

    pub fn has_v2_flag(extensions: BitArray<u16>) -> bool {
        extensions[NodeIdHandshake::V2_FLAG]
    }

    pub fn serialized_size(extensions: BitArray<u16>) -> usize {
        let mut size = 0;
        if Self::is_query(extensions) {
            size += 32
        }
        if Self::is_response(extensions) {
            size += NodeIdHandshakeResponse::serialized_size(extensions);
        }
        size
    }

    pub fn deserialize(stream: &mut dyn Stream, extensions: BitArray<u16>) -> Result<Self> {
        let query = if NodeIdHandshake::is_query(extensions) {
            let mut cookie = [0u8; 32];
            stream.read_bytes(&mut cookie, 32)?;
            Some(NodeIdHandshakeQuery { cookie })
        } else {
            None
        };
        let response = if NodeIdHandshake::is_response(extensions) {
            Some(NodeIdHandshakeResponse::deserialize(stream, extensions)?)
        } else {
            None
        };
        Ok(Self {
            query,
            response,
            is_v2: Self::has_v2_flag(extensions),
        })
    }

    pub fn create_test_query() -> Self {
        let query = NodeIdHandshakeQuery { cookie: [42; 32] };
        Self {
            query: Some(query),
            response: None,
            is_v2: true,
        }
    }

    pub fn create_test_response_v1() -> Self {
        let response = NodeIdHandshakeResponse {
            node_id: PublicKey::from(1),
            signature: Signature::from_bytes([42; 64]),
            v2: None,
        };
        Self {
            query: None,
            response: Some(response),
            is_v2: false,
        }
    }

    pub fn create_test_response_v2() -> Self {
        let response = NodeIdHandshakeResponse {
            node_id: PublicKey::from(1),
            signature: Signature::from_bytes([42; 64]),
            v2: Some(V2Payload {
                salt: [7; 32],
                genesis: BlockHash::from(3),
            }),
        };
        Self {
            query: None,
            response: Some(response),
            is_v2: true,
        }
    }
}

impl Serialize for NodeIdHandshake {
    fn serialize(&self, stream: &mut dyn Stream) -> Result<()> {
        if let Some(query) = &self.query {
            stream.write_bytes(&query.cookie)?;
        }
        if let Some(response) = &self.response {
            response.serialize(stream)?;
        }
        Ok(())
    }

    fn serialize_safe(&self, writer: &mut dyn BufferWriter) {
        if let Some(query) = &self.query {
            writer.write_bytes_safe(&query.cookie);
        }
        if let Some(response) = &self.response {
            response.serialize_safe(writer);
        }
    }
}

impl MessageVariant for NodeIdHandshake {
    fn header_extensions(&self, _payload_len: u16) -> BitArray<u16> {
        let mut extensions = BitArray::default();
        extensions.set(NodeIdHandshake::QUERY_FLAG, self.query.is_some());
        extensions.set(NodeIdHandshake::RESPONSE_FLAG, self.response.is_some());
        extensions.set(Self::V2_FLAG, self.is_v2);
        extensions
    }
}

impl Display for NodeIdHandshake {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(query) = &self.query {
            write!(f, "\ncookie=")?;
            write_hex_bytes(&query.cookie, f)?;
        }

        if let Some(response) = &self.response {
            write!(
                f,
                "\nresp_node_id={}\nresp_sig={}",
                response.node_id,
                response.signature.encode_hex()
            )?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::{assert_deserializable, Message};

    #[test]
    fn serialize_query() {
        let message = Message::NodeIdHandshake(NodeIdHandshake::create_test_query());
        assert_deserializable(&message);
    }

    #[test]
    fn serialize_response_v1() {
        let message = Message::NodeIdHandshake(NodeIdHandshake::create_test_response_v1());
        assert_deserializable(&message);
    }

    #[test]
    fn serialize_response_v2() {
        let message = Message::NodeIdHandshake(NodeIdHandshake::create_test_response_v2());
        assert_deserializable(&message);
    }

    #[test]
    fn valid_v1_signature() {
        let key = KeyPair::new();
        let mut response = NodeIdHandshakeResponse {
            node_id: key.public_key(),
            signature: Signature::default(),
            v2: None,
        };
        let cookie = [42; 32];

        response.sign(&cookie, &key);

        assert_ne!(response.signature, Signature::default());
        assert!(response.validate(&cookie).is_ok());

        // invalid cookie
        assert!(response.validate(&[1; 32]).is_err());

        // invalid node_id
        response.node_id = PublicKey::from(1);
        assert!(response.validate(&cookie).is_err());
    }

    #[test]
    fn valid_v2_signature() {
        let key = KeyPair::new();
        let mut response = NodeIdHandshakeResponse {
            node_id: key.public_key(),
            signature: Signature::default(),
            v2: Some(V2Payload {
                salt: [1; 32],
                genesis: BlockHash::from(3),
            }),
        };
        let cookie = [42; 32];

        response.sign(&cookie, &key);

        assert_ne!(response.signature, Signature::default());
        assert!(response.validate(&cookie).is_ok());

        // invalid cookie
        assert!(response.validate(&[1; 32]).is_err());

        // invalid node_id
        let mut copy = response.clone();
        copy.node_id = PublicKey::from(1);
        assert!(copy.validate(&cookie).is_err());

        // invalid salt
        let mut copy = response.clone();
        copy.v2.as_mut().unwrap().salt = [100; 32];
        assert!(copy.validate(&cookie).is_err());

        // invalid genesis
        let mut copy = response.clone();
        copy.v2.as_mut().unwrap().genesis = BlockHash::from(123);
        assert!(copy.validate(&cookie).is_err());
    }
}
