// .aom writing: MessagePack with struct maps, deflated, in the Alterion
// container. The header's length field is the UNCOMPRESSED payload size,
// which also bounds the inflater on read.

use alloc::vec::Vec;

use serde::Serialize;

use crate::container::{CONTAINER_VERSION, Flags, Header, MAX_P_SIZE};
use crate::error::Error;
use crate::payload::AomPayload;

pub fn write(payload: &AomPayload) -> Result<Vec<u8>, Error> {
    let mut raw = Vec::new();
    let mut ser = rmp_serde::Serializer::new(&mut raw).with_struct_map();
    payload.serialize(&mut ser).map_err(|e| Error::Decode(alloc::format!("{e}")))?;

    let declared = u32::try_from(raw.len())
        .map_err(|_| Error::PayloadTooLarge { declared: u32::MAX, ceiling: MAX_P_SIZE })?;
    if declared > MAX_P_SIZE {
        return Err(Error::PayloadTooLarge { declared, ceiling: MAX_P_SIZE });
    }

    let compressed = miniz_oxide::deflate::compress_to_vec(&raw, 6);
    let header =
        Header { version: CONTAINER_VERSION, flags: Flags::deflate(), payload_len: declared };
    let mut out = Vec::with_capacity(crate::container::HEADER_LENGTH + compressed.len());
    out.extend_from_slice(&header.to_bytes());
    out.extend_from_slice(&compressed);
    Ok(out)
}
