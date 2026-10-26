//! A small model for driving the ABI: the tests load it, and the C host
//! example runs against it end to end. Not part of the library proper.

use ayatsuri_format::model::{
    ArtMesh, Binding, BindingAxis, Model as Rig, Parameter, Part, PhysicsGroup, PhysicsInput,
    PhysicsIo, PhysicsNode, PhysicsOutput, PhysicsRange, PhysicsRig, flags,
};
use ayatsuri_format::payload::{
    Animation, AomPayload, Composition, Curve, CurveTarget, DisplayInfo, Event, Expression,
    ExpressionBlend, ExpressionParam, Interp, Key, Layer, LayerSource,
};

/// A canvas with one parameter that slides a quad right, a second the rig
/// drives, one part and one texture page.
pub fn small_aom() -> Vec<u8> {
    let mut model = Rig::new();
    model.canvas = ayatsuri_format::model::Canvas {
        pixels_per_unit: 1024.0,
        origin_x: 512.0,
        origin_y: 512.0,
        width: 1024.0,
        height: 1024.0,
    };
    model.parameters.push(Parameter {
        id: "ParamA".into(),
        min: -1.0,
        max: 1.0,
        default: 0.0,
        repeat: false,
        decimals: 3,
        blend_shape: false,
    });
    model.parameters.push(Parameter {
        id: "ParamB".into(),
        min: -10.0,
        max: 10.0,
        default: 0.0,
        repeat: false,
        decimals: 3,
        blend_shape: false,
    });
    model.parts.push(Part {
        id: "Part".into(),
        parent_part: None,
        visible: true,
        enabled: true,
        binding: Binding::default(),
        draw_orders: vec![500.0],
        offscreen: None,
    });
    let quad = |dx: f32| vec![dx, 0.0, dx + 0.5, 0.0, dx + 0.5, 0.5, dx, 0.5];
    model.art_meshes.push(ArtMesh {
        id: "Quad".into(),
        parent_part: Some(0),
        parent_deformer: None,
        visible: true,
        enabled: true,
        texture: 0,
        flags: flags::DOUBLE_SIDED,
        blend: Default::default(),
        vertex_count: 4,
        uvs: vec![0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0],
        indices: vec![0, 1, 2, 0, 2, 3],
        masks: Vec::new(),
        binding: Binding {
            curves: Vec::new(),
            axes: vec![BindingAxis { parameter: 0, keys: vec![0.0, 1.0] }],
        },
        opacities: vec![1.0, 1.0],
        draw_orders: vec![500.0, 500.0],
        positions: quad(0.0).into_iter().chain(quad(1.0)).collect(),
        blend_shapes: Vec::new(),
        clip_group: None,
        clip_inherit: false,
        multiply: Vec::new(),
        screen: Vec::new(),
    });
    model.physics = Some(PhysicsRig {
        fps: 60.0,
        gravity_x: 0.0,
        gravity_y: -1.0,
        wind_x: 0.0,
        wind_y: 0.0,
        groups: vec![PhysicsGroup {
            id: "PhysicsSetting1".into(),
            name: "Sway".into(),
            inputs: vec![PhysicsInput {
                param: "ParamA".into(),
                weight: 100.0,
                kind: PhysicsIo::X,
                reflect: false,
            }],
            outputs: vec![PhysicsOutput {
                param: "ParamB".into(),
                vertex: 1,
                scale: 10.0,
                weight: 100.0,
                kind: PhysicsIo::Angle,
                reflect: false,
            }],
            nodes: vec![
                PhysicsNode {
                    x: 0.0,
                    y: 0.0,
                    mobility: 1.0,
                    delay: 1.0,
                    acceleration: 1.0,
                    radius: 0.0,
                },
                PhysicsNode {
                    x: 0.0,
                    y: 5.0,
                    mobility: 0.9,
                    delay: 1.0,
                    acceleration: 1.0,
                    radius: 5.0,
                },
            ],
            norm_position: PhysicsRange { min: -10.0, default: 0.0, max: 10.0 },
            norm_angle: PhysicsRange { min: -10.0, default: 0.0, max: 10.0 },
        }],
    });
    // a one-second scene sliding ParamA from 0 to 1, with an event half way
    let key = |t: f32, v: f32| Key { time: t, value: v, interp: Interp::Linear };
    let wave = Animation {
        name: "wave".into(),
        fps: 30.0,
        duration: 1.0,
        looped: false,
        curves: vec![Curve {
            target: CurveTarget::Parameter,
            id: "ParamA".into(),
            keys: vec![key(0.0, 0.0), key(1.0, 1.0)],
            fade_in: None,
            fade_out: None,
        }],
        events: vec![Event { time: 0.5, value: "clap".into() }],
        ..Default::default()
    };
    // a composition that plays the scene at half speed over two seconds
    let both = Composition {
        name: "both".into(),
        fps: 30.0,
        duration: 2.0,
        looped: false,
        work_area: None,
        layers: vec![Layer {
            source: LayerSource::Scene("wave".into()),
            start: 0.0,
            in_point: 0.0,
            out_point: 1.0,
            speed: 0.5,
            weight: 1.0,
            fade_in: 0.0,
            fade_out: 0.0,
            enabled: true,
            additive: false,
            pan: 0.0,
        }],
    };
    let smile = Expression {
        name: "smile".into(),
        fade_in: Some(0.0),
        fade_out: Some(0.0),
        params: vec![ExpressionParam {
            id: "ParamA".into(),
            value: 0.25,
            blend: ExpressionBlend::Add,
        }],
    };
    let display = DisplayInfo {
        param_names: vec!["Slide".into(), "Sway".into()],
        groups: vec![("Motion".into(), vec![0, 1])],
        part_names: vec!["Body".into()],
        mesh_names: vec!["The quad".into()],
        model_groups: Some(vec![("Parameter".into(), "LipSync".into(), vec!["ParamA".into()])]),
        ..Default::default()
    };
    let payload = AomPayload {
        model,
        textures: vec![serde_bytes::ByteBuf::from(b"not really a png".to_vec())],
        display: Some(display),
        animations: vec![wave],
        compositions: vec![both],
        expressions: vec![smile],
        ..Default::default()
    };
    ayatsuri_format::write(&payload).unwrap_or_default()
}
