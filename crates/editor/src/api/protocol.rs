// The Ayatsuri API protocol: Cubism Editor's External Application
// Integration API, built from Live2D's published documentation
// (docs.live2d.com, "External Application Integration API") so existing
// clients such as VTube Studio connect unchanged. Pure request handling,
// no sockets: the server feeds text in and sends text out.
//
// Envelope: {Version?, Timestamp?, RequestId?, Type, Method, Data}. Errors
// come back as Type "Error" with Data.ErrorType.

use std::collections::HashMap;
use std::time::Instant;

use serde_json::{Map, Value, json};

/// Versions a client may ask for; the published methods span these.
const VERSIONS: &[&str] = &["0.9.0", "0.9.1", "0.9.2", "0.9.3", "1.0.0", "1.0.1"];
/// What GetAPIVersion reports and responses carry by default.
pub(crate) const API_VERSION: &str = "1.0.1";
/// Overrides fall away this long after their last write, as in Cubism.
pub(crate) const OVERRIDE_LIFE_SECS: f32 = 0.5;

/// One parameter as the API lists it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ParamInfo {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) group: String,
    pub(crate) min: f32,
    pub(crate) max: f32,
    pub(crate) default: f32,
    pub(crate) repeat: bool,
}

/// What the editor shows right now, refreshed by the UI thread each frame.
#[derive(Clone, Debug, Default)]
pub(crate) struct Snapshot {
    /// None when no model is open.
    pub(crate) model_uid: Option<String>,
    pub(crate) document_uid: String,
    pub(crate) file_path: String,
    pub(crate) edit_mode: &'static str,
    pub(crate) params: Vec<ParamInfo>,
    pub(crate) values: Vec<f32>,
    /// (GroupUID, GroupName), the parameter palette's folders.
    pub(crate) groups: Vec<(String, String)>,
}

/// One registered client, kept across restarts in the preferences.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct Client {
    pub(crate) name: String,
    pub(crate) token: String,
    pub(crate) allowed: bool,
    /// Shown in the client list; the app's own path when it sends one.
    pub(crate) path: String,
}

/// What one connection knows about itself.
#[derive(Default)]
pub(crate) struct Conn {
    /// The token this connection registered with, if it has.
    pub(crate) token: Option<String>,
    /// Which notify events it subscribed to.
    pub(crate) subscribed: Vec<String>,
}

/// Everything a request may read or change, behind the server's locks.
pub(crate) struct World<'a> {
    pub(crate) snap: &'a Snapshot,
    pub(crate) clients: &'a mut Vec<Client>,
    /// Parameter id -> (value, when it was written).
    pub(crate) overrides: &'a mut HashMap<String, (f32, Instant)>,
    /// Set when a new client asks to be allowed, for the editor's prompt.
    pub(crate) wants_approval: &'a mut Vec<String>,
}

const NOTIFY: &[&str] = &[
    "NotifyMocFileExported",
    "NotifyPhysicsFileExported",
    "NotifyMotionFileExported",
    "NotifyMotionSyncFileExported",
    "NotifyChangeEditMode",
];

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// A message in the published envelope.
pub(crate) fn envelope(
    kind: &str,
    method: &str,
    request_id: Option<&Value>,
    data: Value,
) -> String {
    let mut m = Map::new();
    m.insert("Version".into(), API_VERSION.into());
    m.insert("Timestamp".into(), now_ms().into());
    if let Some(id) = request_id {
        m.insert("RequestId".into(), id.clone());
    }
    m.insert("Type".into(), kind.into());
    m.insert("Method".into(), method.into());
    m.insert("Data".into(), data);
    Value::Object(m).to_string()
}

fn error(method: &str, request_id: Option<&Value>, kind: &str) -> String {
    envelope("Error", method, request_id, json!({ "ErrorType": kind }))
}

/// A value the docs show both as a number and as a string ("0.0").
fn number(v: &Value) -> Option<f32> {
    match v {
        Value::Number(n) => n.as_f64().map(|f| f as f32),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
    .filter(|f: &f32| f.is_finite())
}

fn new_token() -> String {
    // not a secret worth more than the user's click: it only lets an app
    // skip the approval prompt it already passed once
    let mut seed = now_ms() ^ (std::process::id() as u64).rotate_left(32);
    let mut out = String::with_capacity(32);
    for _ in 0..32 {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        out.push(char::from_digit((seed & 15) as u32, 16).unwrap_or('0'));
    }
    out
}

/// Answer one text frame. Returns the reply, or None for nothing to say.
pub(crate) fn handle(text: &str, conn: &mut Conn, w: &mut World) -> Option<String> {
    let Ok(req) = serde_json::from_str::<Value>(text) else {
        return Some(error("", None, "InvalidJson"));
    };
    let method = req.get("Method").and_then(Value::as_str).unwrap_or("").to_string();
    let rid = req.get("RequestId");
    if let Some(v) = req.get("Version").and_then(Value::as_str)
        && !VERSIONS.contains(&v)
    {
        return Some(error(&method, rid, "UnsupportedVersion"));
    }
    if req.get("Type").and_then(Value::as_str) != Some("Request") {
        return Some(error(&method, rid, "InvalidType"));
    }
    let empty = Map::new();
    let data = req.get("Data").and_then(Value::as_object).unwrap_or(&empty);
    let ok = |d: Value| Some(envelope("Response", &method, rid, d));
    let err = |kind: &str| Some(error(&method, rid, kind));

    // these work before the user has allowed the client
    match method.as_str() {
        "RegisterPlugin" => {
            let Some(name) = data.get("Name").and_then(Value::as_str).filter(|n| !n.is_empty())
            else {
                return err("InvalidData");
            };
            let path = data.get("Path").and_then(Value::as_str).unwrap_or("").to_string();
            let sent = data.get("Token").and_then(Value::as_str);
            // a known, allowed token keeps working; anything else is a new
            // registration waiting for the user
            let known = sent.and_then(|t| w.clients.iter().position(|c| c.token == t));
            let token = match known {
                Some(i) if w.clients.get(i).is_some_and(|c| c.allowed) => {
                    sent.unwrap_or_default().to_string()
                }
                _ => {
                    let token = new_token();
                    w.clients.retain(|c| Some(c.token.as_str()) != sent);
                    w.clients.push(Client {
                        name: name.to_string(),
                        token: token.clone(),
                        allowed: false,
                        path,
                    });
                    w.wants_approval.push(token.clone());
                    token
                }
            };
            conn.token = Some(token.clone());
            return ok(json!({ "Token": token }));
        }
        "GetIsApproval" => {
            let allowed = conn
                .token
                .as_ref()
                .is_some_and(|t| w.clients.iter().any(|c| &c.token == t && c.allowed));
            return ok(json!({ "Result": allowed }));
        }
        "GetAPIVersion" => return ok(json!({ "Version": API_VERSION })),
        "SetGlobalVersion" => return ok(json!({})),
        _ => {}
    }
    // subscriptions are kept even before the user allows the app (clients
    // subscribe right after registering); events only flow once allowed
    if let Some(event) = NOTIFY.iter().find(|e| **e == method) {
        let on = data
            .get("Enabled")
            .or_else(|| data.get("Enable"))
            .and_then(Value::as_bool)
            .unwrap_or(true);
        conn.subscribed.retain(|s| s != event);
        if on {
            conn.subscribed.push((*event).to_string());
        }
        return ok(json!({ "Accepted": true }));
    }

    let allowed =
        conn.token.as_ref().is_some_and(|t| w.clients.iter().any(|c| &c.token == t && c.allowed));
    if !allowed {
        return err("PluginNotRegistered");
    }

    let snap = w.snap;
    // a request naming a model must name the open one
    let model_ok = |d: &Map<String, Value>| {
        let uid = snap.model_uid.as_deref();
        match (
            d.get("ModelUID").and_then(Value::as_str),
            d.get("DocumentUID").and_then(Value::as_str),
        ) {
            (Some(m), _) => uid == Some(m),
            (None, Some(doc)) => uid.is_some() && doc == snap.document_uid,
            (None, None) => false,
        }
    };

    match method.as_str() {
        "GetCurrentModelUID" => match &snap.model_uid {
            Some(uid) => ok(json!({ "ModelUID": uid })),
            None => err("InvalidModel"),
        },
        "GetCurrentDocumentUID" => match &snap.model_uid {
            Some(_) => ok(json!({ "DocumentUID": snap.document_uid })),
            None => err("InvalidDocument"),
        },
        "GetCurrentEditMode" => ok(json!({ "EditMode": snap.edit_mode })),
        "GetDocuments" | "GetDocument" => {
            let (mut phys, mut modeling, mut anim) = (vec![], vec![], vec![]);
            if let Some(uid) = &snap.model_uid {
                if method == "GetDocument"
                    && data.get("DocumentUID").and_then(Value::as_str)
                        != Some(snap.document_uid.as_str())
                {
                    return err("InvalidDocument");
                }
                let doc = |extra: Value| {
                    let mut o = json!({
                        "DocumentUID": snap.document_uid,
                        "DocumentFilePath": snap.file_path,
                    });
                    if let (Some(o), Some(e)) = (o.as_object_mut(), extra.as_object()) {
                        o.extend(e.clone());
                    }
                    o
                };
                match snap.edit_mode {
                    "Physics" => phys.push(doc(json!({ "ModelUID": uid }))),
                    "Animation" => anim.push(doc(json!({
                        "Views": [{ "ModelUID": uid, "ModelFilePath": snap.file_path }]
                    }))),
                    _ => modeling.push(doc(json!({ "Views": [{ "ModelUID": uid }] }))),
                }
            }
            ok(json!({
                "PhysicsDocuments": phys,
                "ModelingDocuments": modeling,
                "AnimationDocuments": anim,
            }))
        }
        "GetParameters" => {
            if !model_ok(data) {
                return err("InvalidModel");
            }
            let list: Vec<Value> = snap
                .params
                .iter()
                .map(|p| {
                    json!({
                        "Id": p.id, "Name": p.name, "GroupUID": p.group,
                        "Default": p.default, "Max": p.max, "Min": p.min,
                        "Repeat": p.repeat, "Type": 0,
                    })
                })
                .collect();
            ok(json!({ "Parameters": list }))
        }
        "GetParameterGroups" => {
            if !model_ok(data) {
                return err("InvalidModel");
            }
            let list: Vec<Value> = snap
                .groups
                .iter()
                .map(|(uid, name)| json!({ "GroupUID": uid, "GroupName": name }))
                .collect();
            ok(json!({ "Groups": list }))
        }
        "GetParameterValues" => {
            if !model_ok(data) {
                return err("InvalidModel");
            }
            let wanted: Option<Vec<&str>> = data
                .get("Ids")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).collect());
            let mut list = Vec::new();
            for (p, v) in snap.params.iter().zip(&snap.values) {
                if wanted.as_ref().is_none_or(|w| w.contains(&p.id.as_str())) {
                    list.push(json!({ "Id": p.id, "Value": v }));
                }
            }
            ok(json!({ "Parameters": list }))
        }
        "SetParameterValues" => {
            if !model_ok(data) {
                return err("InvalidModel");
            }
            let Some(items) = data.get("Parameters").and_then(Value::as_array) else {
                return err("InvalidData");
            };
            let now = Instant::now();
            for item in items {
                let (Some(id), Some(v)) =
                    (item.get("Id").and_then(Value::as_str), item.get("Value").and_then(number))
                else {
                    return err("InvalidParameter");
                };
                let Some(p) = snap.params.iter().find(|p| p.id == id) else {
                    return err("InvalidParameter");
                };
                // a temporary override, clamped like the slider would be
                let v = if p.repeat { v } else { v.clamp(p.min, p.max) };
                // a bounded buffer: only the model's own parameters get in
                w.overrides.insert(p.id.clone(), (v, now));
            }
            ok(json!({}))
        }
        "ClearParameterValues" => {
            if !model_ok(data) {
                return err("InvalidModel");
            }
            w.overrides.clear();
            ok(json!({}))
        }
        "SendCubismLog" => {
            let msg: String = data
                .get("Message")
                .and_then(Value::as_str)
                .unwrap_or("")
                .chars()
                .take(5000)
                .collect();
            let client = conn
                .token
                .as_ref()
                .and_then(|t| w.clients.iter().find(|c| &c.token == t))
                .map(|c| c.name.clone())
                .unwrap_or_default();
            match data.get("Type").and_then(Value::as_str) {
                Some("warning") => log::warn!("api {client}: {msg}"),
                _ => log::info!("api {client}: {msg}"),
            }
            ok(json!({}))
        }
        "GetPhysicsInfo" => {
            if !model_ok(data) {
                return err("InvalidModel");
            }
            ok(json!({}))
        }
        _ => err("MethodNotFound"),
    }
}

/// An event for subscribers, in the published envelope (no RequestId, so it
/// can never be mistaken for the answer to a pending request).
pub(crate) fn event(method: &str, data: Value) -> String {
    envelope("Event", method, None, data)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> Snapshot {
        Snapshot {
            model_uid: Some("1001".into()),
            document_uid: "2001".into(),
            file_path: "/m/ariu.aom".into(),
            edit_mode: "Modeling",
            params: vec![ParamInfo {
                id: "ParamAngleX".into(),
                name: "Angle X".into(),
                group: "ParamGroup0".into(),
                min: -30.0,
                max: 30.0,
                default: 0.0,
                repeat: false,
            }],
            values: vec![5.0],
            groups: vec![("ParamGroup0".into(), "Face".into())],
        }
    }

    struct Rig {
        snap: Snapshot,
        clients: Vec<Client>,
        overrides: HashMap<String, (f32, Instant)>,
        asks: Vec<String>,
        conn: Conn,
    }

    impl Rig {
        fn new() -> Self {
            Rig {
                snap: snapshot(),
                clients: Vec::new(),
                overrides: HashMap::new(),
                asks: Vec::new(),
                conn: Conn::default(),
            }
        }
        fn send(&mut self, text: &str) -> Value {
            let mut w = World {
                snap: &self.snap,
                clients: &mut self.clients,
                overrides: &mut self.overrides,
                wants_approval: &mut self.asks,
            };
            serde_json::from_str(&handle(text, &mut self.conn, &mut w).unwrap()).unwrap()
        }
        fn call(&mut self, method: &str, data: Value) -> Value {
            self.send(
                &json!({"Version":"1.0.0","RequestId":"7","Type":"Request",
                "Method":method,"Data":data})
                .to_string(),
            )
        }
    }

    #[test]
    fn register_then_wait_for_the_user() {
        let mut r = Rig::new();
        let reg = r.call("RegisterPlugin", json!({"Name":"VTube Studio","Token":null}));
        let token = reg["Data"]["Token"].as_str().unwrap().to_string();
        assert_eq!(reg["Type"], "Response");
        assert_eq!(reg["RequestId"], "7");
        assert_eq!(r.asks, vec![token.clone()]);
        assert_eq!(r.call("GetIsApproval", json!({}))["Data"]["Result"], false);
        // nothing else works until the user allows it
        let blocked = r.call("GetCurrentModelUID", json!({}));
        assert_eq!(blocked["Type"], "Error");
        assert_eq!(blocked["Data"]["ErrorType"], "PluginNotRegistered");
        r.clients[0].allowed = true;
        assert_eq!(r.call("GetIsApproval", json!({}))["Data"]["Result"], true);
        assert_eq!(r.call("GetCurrentModelUID", json!({}))["Data"]["ModelUID"], "1001");
        // a reconnect with the allowed token skips the prompt
        let mut again = Rig { clients: r.clients.clone(), ..Rig::new() };
        let back = again.call("RegisterPlugin", json!({"Name":"VTube Studio","Token":token}));
        assert_eq!(back["Data"]["Token"], token.as_str());
        assert!(again.asks.is_empty());
    }

    fn allowed() -> Rig {
        let mut r = Rig::new();
        r.call("RegisterPlugin", json!({"Name":"t"}));
        r.clients[0].allowed = true;
        r
    }

    #[test]
    fn parameters_list_read_and_override() {
        let mut r = allowed();
        let list = r.call("GetParameters", json!({"ModelUID":"1001"}));
        assert_eq!(list["Data"]["Parameters"][0]["Id"], "ParamAngleX");
        assert_eq!(list["Data"]["Parameters"][0]["Min"], -30.0);
        let vals = r.call("GetParameterValues", json!({"ModelUID":"1001"}));
        assert_eq!(vals["Data"]["Parameters"][0]["Value"], 5.0);
        // values may come as strings, and clamp to the range
        let set = r.call(
            "SetParameterValues",
            json!({"ModelUID":"1001","Parameters":[{"Id":"ParamAngleX","Value":"99"}]}),
        );
        assert_eq!(set["Type"], "Response");
        assert_eq!(r.overrides["ParamAngleX"].0, 30.0);
        let bad = r.call(
            "SetParameterValues",
            json!({"ModelUID":"1001","Parameters":[{"Id":"Nope","Value":1}]}),
        );
        assert_eq!(bad["Data"]["ErrorType"], "InvalidParameter");
        let wrong = r.call("GetParameters", json!({"ModelUID":"999"}));
        assert_eq!(wrong["Data"]["ErrorType"], "InvalidModel");
        r.call("ClearParameterValues", json!({"ModelUID":"1001"}));
        assert!(r.overrides.is_empty());
    }

    #[test]
    fn envelope_errors() {
        let mut r = allowed();
        assert_eq!(r.send("not json")["Data"]["ErrorType"], "InvalidJson");
        let v = r.send(r#"{"Version":"9.9.9","Type":"Request","Method":"X","Data":{}}"#);
        assert_eq!(v["Data"]["ErrorType"], "UnsupportedVersion");
        let t = r.send(r#"{"Type":"Event","Method":"X","Data":{}}"#);
        assert_eq!(t["Data"]["ErrorType"], "InvalidType");
        assert_eq!(r.call("Nope", json!({}))["Data"]["ErrorType"], "MethodNotFound");
        // no RequestId sent, none echoed
        let quiet = r.send(r#"{"Type":"Request","Method":"GetAPIVersion","Data":{}}"#);
        assert!(quiet.get("RequestId").is_none());
    }

    #[test]
    fn subscribing_before_approval_is_kept() {
        let mut r = Rig::new();
        r.call("RegisterPlugin", json!({"Name":"VTube Studio"}));
        // VTube Studio subscribes straight after registering
        let sub = r.call("NotifyChangeEditMode", json!({"Enabled":true}));
        assert_eq!(sub["Data"]["Accepted"], true);
        assert_eq!(r.conn.subscribed, vec!["NotifyChangeEditMode"]);
    }

    #[test]
    fn subscriptions_and_documents() {
        let mut r = allowed();
        let sub = r.call("NotifyChangeEditMode", json!({"Enabled":true}));
        assert_eq!(sub["Data"]["Accepted"], true);
        assert_eq!(r.conn.subscribed, vec!["NotifyChangeEditMode"]);
        r.call("NotifyChangeEditMode", json!({"Enabled":false}));
        assert!(r.conn.subscribed.is_empty());
        let docs = r.call("GetDocuments", json!({}));
        assert_eq!(docs["Data"]["ModelingDocuments"][0]["Views"][0]["ModelUID"], "1001");
        assert_eq!(docs["Data"]["PhysicsDocuments"].as_array().unwrap().len(), 0);
    }
}
