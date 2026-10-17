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
    let mut out: AomPayload =
        rmp_serde::from_slice(plain).map_err(|e| Error::Decode(alloc::format!("{e}")))?;
    // scenes from older files carry straight-line tracks; curves from here on
    for a in &mut out.animations {
        a.upgrade();
    }
    Ok(out)
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
            meta: Meta {
                name: "t".into(),
                author: String::new(),
                tool: "test".into(),
                timelapse: None,
            },
            model,
            textures: vec![serde_bytes::ByteBuf::from(vec![1u8, 2, 3])],
            display: None,
            animations: Vec::new(),
            compositions: Vec::new(),
            expressions: vec![crate::Expression {
                name: "smile".into(),
                fade_in: Some(0.5),
                fade_out: None,
                params: vec![crate::ExpressionParam {
                    id: "ParamEyeLOpen".into(),
                    value: 0.6,
                    blend: crate::ExpressionBlend::Multiply,
                }],
            }],
        };
        let bytes = crate::writer::write(&payload).unwrap();
        let back = crate::read::read(&bytes).unwrap();
        assert_eq!(back.meta.name, "t");
        assert_eq!(back.model.parameters.len(), 1);
        let m = &back.model.art_meshes[0];
        assert_eq!(m.positions, vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0]);
        assert_eq!(m.indices, vec![0, 1, 2]);
        assert_eq!(back.textures[0].as_ref(), &[1u8, 2, 3]);
        assert_eq!(back.expressions, payload.expressions);
    }

    #[test]
    fn old_straight_line_tracks_become_linear_curves() {
        use crate::payload::{Animation, CurveTarget, Interp, Track};
        let old = AomPayload {
            animations: vec![Animation {
                name: "idle".into(),
                fps: 30.0,
                duration: 1.0,
                tracks: vec![Track {
                    param: "ParamX".into(),
                    points: vec![(0.0, 0.0), (1.0, 5.0)],
                }],
                ..Animation::default()
            }],
            ..AomPayload::default()
        };
        let back = crate::read::read(&crate::writer::write(&old).unwrap()).unwrap();
        let a = &back.animations[0];
        assert!(a.tracks.is_empty());
        let c = &a.curves[0];
        assert_eq!((c.target, c.id.as_str()), (CurveTarget::Parameter, "ParamX"));
        assert_eq!(c.keys.len(), 2);
        assert_eq!(c.keys[1].value, 5.0);
        assert!(c.keys.iter().all(|k| k.interp == Interp::Linear));
    }

    #[test]
    fn compositions_ride_along_and_old_files_have_none() {
        use crate::payload::{Composition, Layer, LayerSource};
        let with = AomPayload {
            compositions: vec![Composition {
                name: "intro".into(),
                fps: 30.0,
                duration: 4.0,
                layers: vec![Layer {
                    source: LayerSource::Scene("idle".into()),
                    out_point: 3.0,
                    fade_in: 0.5,
                    ..Layer::default()
                }],
                ..Composition::default()
            }],
            ..AomPayload::default()
        };
        let back = crate::read::read(&crate::writer::write(&with).unwrap()).unwrap();
        assert_eq!(back.compositions, with.compositions);
        let old =
            crate::read::read(&crate::writer::write(&AomPayload::default()).unwrap()).unwrap();
        assert!(old.compositions.is_empty());
    }

    #[test]
    fn rejects_garbage() {
        assert!(crate::read::read(b"not an aom").is_err());
    }
}
