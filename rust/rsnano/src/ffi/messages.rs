use std::{ffi::c_void, sync::Arc};

use num::FromPrimitive;

use crate::{
    messages::{MessageHeader, MessageType},
    NetworkConstants,
};

use super::{FfiStream, NetworkConstantsDto, StringDto};

#[no_mangle]
pub unsafe extern "C" fn rsn_message_type_to_string(msg_type: u8, result: *mut StringDto) {
    (*result) = match MessageType::from_u8(msg_type) {
        Some(msg_type) => msg_type.as_str().into(),
        None => "n/a".into(),
    }
}

pub struct MessageHeaderHandle(MessageHeader);

#[no_mangle]
pub unsafe extern "C" fn rsn_message_header_create(
    constants: *const NetworkConstantsDto,
    message_type: u8,
    version_using: i16,
) -> *mut MessageHeaderHandle {
    let message_type = MessageType::from_u8(message_type).unwrap();
    let constants = Arc::new(NetworkConstants::try_from(&*constants).unwrap());
    let header = if version_using < 0 {
        MessageHeader::new(constants, message_type)
    } else {
        MessageHeader::with_version_using(constants, message_type, version_using as u8)
    };
    Box::into_raw(Box::new(MessageHeaderHandle(header)))
}

#[no_mangle]
pub unsafe extern "C" fn rsn_message_header_empty() -> *mut MessageHeaderHandle {
    let message_type = MessageType::Invalid;
    let constants = Arc::new(NetworkConstants::empty());
    let header = MessageHeader::new(constants, message_type);
    Box::into_raw(Box::new(MessageHeaderHandle(header)))
}

#[no_mangle]
pub unsafe extern "C" fn rsn_message_header_clone(
    handle: *mut MessageHeaderHandle,
) -> *mut MessageHeaderHandle {
    Box::into_raw(Box::new(MessageHeaderHandle((*handle).0.clone())))
}

#[no_mangle]
pub unsafe extern "C" fn rsn_message_header_destroy(handle: *mut MessageHeaderHandle) {
    drop(Box::from_raw(handle))
}

#[no_mangle]
pub unsafe extern "C" fn rsn_message_header_version_using(handle: *mut MessageHeaderHandle) -> u8 {
    (*handle).0.version_using()
}

#[no_mangle]
pub unsafe extern "C" fn rsn_message_header_size() -> usize {
    MessageHeader::size()
}

#[no_mangle]
pub unsafe extern "C" fn rsn_message_header_deserialize(
    handle: *mut MessageHeaderHandle,
    stream: *mut c_void,
) -> bool {
    let mut stream = FfiStream::new(stream);
    (*handle).0.deserialize(&mut stream).is_ok()
}
