// .aom reading: parse the container, inflate within the declared bound,
// decode the MessagePack payload. Files are untrusted; every failure is a
// typed error, and the inflater can never run past the declared length.

use alloc::vec::Vec;

use crate::container::Header;
use crate::error::Error;
use crate::payload::AomPayload;

pub fn read(bytes: &[u8]) -> Result<AomPayload, Error> {
    let (header, payload) = Header::parse(bytes)?;
    let raw: Vec<u8>;
    let plain = if header.flags.compressed() {
        raw = miniz_oxide::inflate::decompress_to_vec_with_limit(
            payload,
            header.payload_len as usize,
        )
        .map_err(|e| Error::Inflate(alloc::format!("{e}")))?;
        if raw.len() != header.payload_len as usize {
            return Err(Error::InflateShort { declared: header.payload_len, produced: raw.len() });
        }
        raw.as_slice()
    } else {
        payload
    };
    rmp_serde::from_slice(plain).map_err(|e| Error::Decode(alloc::format!("{e}")))
}

#[cfg(test)]
mod tests {
    use crate::model::{ArtMesh, Binding, Model, Parameter};
    use crate::payload::{AomPayload, Meta};

    #[test]
    fn aom_round_trips() {
        let mut model = Model::new();
        model.parameters.push(Parameter {
            id: "ParamX".into(),
            min: 0.0,
            max: 1.0,
            default: 0.5,
            repeat: false,
            decimals: 3,
        });
        model.art_meshes.push(ArtMesh {
            id: "M".into(),
            parent_part: None,
            parent_deformer: None,
            visible: true,
            enabled: true,
            texture: 0,
            flags: 0,
            vertex_count: 3,
            uvs: vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
            indices: vec![0, 1, 2],
            masks: vec![],
            binding: Binding::default(),
            opacities: vec![1.0],
            draw_orders: vec![500.0],
            positions: vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
        });
        let payload = AomPayload {
            meta: Meta { name: "t".into(), author: String::new(), tool: "test".into() },
            model,
            textures: vec![serde_bytes::ByteBuf::from(vec![1u8, 2, 3])],
            display: None,
            animations: Vec::new(),
        };
        let bytes = crate::writer::write(&payload).unwrap();
        let back = crate::read::read(&bytes).unwrap();
        assert_eq!(back.meta.name, "t");
        assert_eq!(back.model.parameters.len(), 1);
        let m = &back.model.art_meshes[0];
        assert_eq!(m.positions, vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0]);
        assert_eq!(m.indices, vec![0, 1, 2]);
        assert_eq!(back.textures[0].as_ref(), &[1u8, 2, 3]);
    }

    #[test]
    fn rejects_garbage() {
        assert!(crate::read::read(b"not an aom").is_err());
    }
}
