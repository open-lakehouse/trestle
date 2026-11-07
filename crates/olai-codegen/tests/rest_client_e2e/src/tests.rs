use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::extract::{OriginalUri, Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use futures::TryStreamExt;
use serde_json::{Value, json};

use crate::codegen::DemoClient;
use crate::codegen::error::Error;
use crate::models::demo::v1::{Color, CreateWidgetRequest, ErrorCode, Widget};

/// What the server saw: paths, bodies and headers.
#[derive(Clone, Default)]
struct Seen(Arc<Mutex<Vec<String>>>);

impl Seen {
    fn push(&self, entry: impl Into<String>) {
        self.0.lock().unwrap().push(entry.into());
    }

    fn all(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

fn record(seen: &Seen, uri: &OriginalUri, headers: &HeaderMap) {
    seen.push(format!("path {}", uri.path()));
    if let Some(auth) = headers.get(header::AUTHORIZATION) {
        seen.push(format!("auth {}", auth.to_str().unwrap()));
    }
    if let Some(ct) = headers.get(header::CONTENT_TYPE) {
        seen.push(format!("content-type {}", ct.to_str().unwrap()));
    }
    if let Some(agent) = headers.get(header::USER_AGENT) {
        seen.push(format!("user-agent {}", agent.to_str().unwrap()));
    }
}

async fn get_widget(
    State(seen): State<Seen>,
    uri: OriginalUri,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Response {
    record(&seen, &uri, &headers);
    match name.as_str() {
        "missing" => (
            StatusCode::NOT_FOUND,
            axum::Json(json!({
                "error_code": "WIDGET_NOT_FOUND",
                "message": "no widget named missing",
                "details": [{"reason": "gone"}],
            })),
        )
            .into_response(),
        "odd" => (
            StatusCode::BAD_REQUEST,
            axum::Json(json!({"errorCode": "SOMETHING_NEW", "message": "odd request"})),
        )
            .into_response(),
        "text" => (StatusCode::BAD_GATEWAY, "bad gateway\n").into_response(),
        "nested" => (
            StatusCode::CONFLICT,
            axum::Json(json!({
                "error": {"message": "table exists", "type": "AlreadyExistsException", "code": 409}
            })),
        )
            .into_response(),
        "oauth" => (
            StatusCode::BAD_REQUEST,
            axum::Json(json!({"error": "invalid_grant", "error_description": "token expired"})),
        )
            .into_response(),
        "json" => axum::Json(json!({
            "name": name,
            "attributes": {"count": 3, "ratio": 0.5, "nested": {"id": 7}},
            "extra": [1, "two", null, true],
            "notes": [{"a": 1}, 2.5],
            "rows": {"t": [1, 2]},
        }))
        .into_response(),
        _ => axum::Json(json!({
            "name": name,
            "color": "PURPLE",
            "created_at": 1_700_000_000_000_i64,
            "size_bytes": 12,
            "properties": {"a": "1"},
            "display_name": "Shown",
            "metadata-location": "s3://bucket/m",
        }))
        .into_response(),
    }
}

async fn create_widget(
    State(seen): State<Seen>,
    uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    record(&seen, &uri, &headers);
    let body = String::from_utf8(body.to_vec()).unwrap();
    seen.push(format!("body {body}"));
    let request: Value = serde_json::from_str(&body).unwrap();
    axum::Json(json!({
        "name": request["name"],
        "created_at": request["created_at"],
        "type": request["type"],
    }))
    .into_response()
}

async fn list_widgets(State(seen): State<Seen>, uri: OriginalUri, headers: HeaderMap) -> Response {
    record(&seen, &uri, &headers);
    if let Some(query) = uri.query() {
        seen.push(format!("query {query}"));
    }
    let page = if uri.query().is_some_and(|q| q.contains("page_token=p2")) {
        json!({"widgets": [{"name": "w3"}]})
    } else {
        json!({"widgets": [{"name": "w1"}, {"name": "w2"}], "next_page_token": "p2"})
    };
    axum::Json(page).into_response()
}

async fn delete_widget(State(seen): State<Seen>, uri: OriginalUri, headers: HeaderMap) -> Response {
    record(&seen, &uri, &headers);
    StatusCode::OK.into_response()
}

async fn exchange_token(
    State(seen): State<Seen>,
    uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    record(&seen, &uri, &headers);
    seen.push(format!(
        "body {}",
        String::from_utf8(body.to_vec()).unwrap()
    ));
    axum::Json(json!({"access_token": "issued", "expires_in": 3600})).into_response()
}

async fn widget_exists(State(seen): State<Seen>, Path(name): Path<String>) -> StatusCode {
    seen.push(format!("HEAD {name}"));
    match name.as_str() {
        "missing" => StatusCode::NOT_FOUND,
        "denied" => StatusCode::FORBIDDEN,
        _ => StatusCode::OK,
    }
}

async fn search_widgets(State(seen): State<Seen>, body: Bytes) -> Response {
    let body = String::from_utf8(body.to_vec()).unwrap();
    seen.push(format!("GET body {body}"));
    let prefix = serde_json::from_str::<Value>(&body).unwrap()["prefix"]
        .as_str()
        .unwrap()
        .to_string();
    let widgets: Vec<Value> = ["w1", "w2", "x1"]
        .into_iter()
        .filter(|name| name.starts_with(&prefix))
        .map(|name| json!({"name": name}))
        .collect();
    axum::Json(json!({"widgets": widgets})).into_response()
}

async fn tag_widget(State(seen): State<Seen>, Path(name): Path<String>, body: Bytes) -> Response {
    let body = String::from_utf8(body.to_vec()).unwrap();
    seen.push(format!("tag {name} body {body}"));
    StatusCode::OK.into_response()
}

async fn rename_widget(
    State(seen): State<Seen>,
    Path(name): Path<String>,
    body: Bytes,
) -> Response {
    let body = String::from_utf8(body.to_vec()).unwrap();
    seen.push(format!("rename {name} body {body}"));
    StatusCode::OK.into_response()
}

async fn stream_widgets(uri: OriginalUri) -> Response {
    let prefix = uri
        .query()
        .and_then(|q| q.strip_prefix("prefix="))
        .unwrap_or_default();
    let lines: String = ["w1", "w2", "x1"]
        .into_iter()
        .filter(|name| name.starts_with(prefix))
        .map(|name| format!("{}\n\n", json!({"name": name, "created_at": 1})))
        .collect();
    ([(header::CONTENT_TYPE, "application/x-ndjson")], lines).into_response()
}

async fn widget_version(Path(name): Path<String>) -> Response {
    let body = if name == "bare" {
        String::new()
    } else {
        json!({"name": name}).to_string()
    };
    ([("Widget-Version", "7"), ("ETag", "abc")], body).into_response()
}

async fn put_blob(
    State(seen): State<Seen>,
    uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    seen.push(format!(
        "PUT {} {content_type} {:?}",
        uri.path(),
        body.as_ref()
    ));
    StatusCode::NO_CONTENT
}

async fn get_blob(Path(path): Path<String>, headers: HeaderMap) -> Vec<u8> {
    match headers.get(header::RANGE).and_then(|v| v.to_str().ok()) {
        Some(range) => format!("range:{range}").into_bytes(),
        None => format!("blob:{path}").into_bytes(),
    }
}

/// Start the demo server on a free port; returns its base URL and what it saw.
async fn serve() -> (url::Url, Seen) {
    let seen = Seen::default();
    let app = Router::new()
        .route("/api/v1/widgets", get(list_widgets).post(create_widget))
        .route(
            "/api/v1/widgets/{name}",
            get(get_widget).delete(delete_widget).head(widget_exists),
        )
        .route("/api/v1/search", get(search_widgets))
        .route("/api/v1/widgets/{name}/tags", post(tag_widget))
        .route("/api/v1/widgets/{name}/rename", post(rename_widget))
        .route("/api/v1/widget-lines", get(stream_widgets))
        .route("/api/v1/widgets/{name}/version", get(widget_version))
        .route("/api/v1/blobs/{*path}", put(put_blob).get(get_blob))
        .route("/auth/tokens", post(exchange_token))
        .with_state(seen.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}/").parse().unwrap(), seen)
}

#[tokio::test]
async fn path_values_are_percent_encoded() {
    let (base, seen) = serve().await;
    let client = DemoClient::new_unauthenticated(base);
    let widget = client.widget("a/b?c d").get().await.unwrap();
    assert_eq!(widget.name, "a/b?c d");
    assert!(
        seen.all()
            .contains(&"path /api/v1/widgets/a%2Fb%3Fc%20d".to_string()),
        "{:?}",
        seen.all()
    );
}

#[tokio::test]
async fn responses_read_numbers_names_and_unknown_enums() {
    let (base, _) = serve().await;
    let client = DemoClient::new_unauthenticated(base);
    let widget: Widget = client.widget("w").get().await.unwrap();
    // `PURPLE` is not a `Color`: `ignore_unknown_enum_values` makes it the zero value.
    assert_eq!(
        widget.color,
        buffa::EnumValue::Known(Color::COLOR_UNSPECIFIED)
    );
    assert_eq!(widget.created_at, Some(1_700_000_000_000));
    assert_eq!(widget.size_bytes, 12);
    assert_eq!(widget.display_name, "Shown");
    assert_eq!(widget.metadata_location, "s3://bucket/m");
    assert_eq!(widget.properties.get("a").map(String::as_str), Some("1"));
}

#[tokio::test]
async fn requests_write_snake_case_and_int64_numbers() {
    let (base, seen) = serve().await;
    let client = DemoClient::new_unauthenticated(base);
    let created = client
        .create_widget("w")
        .with_created_at(1_700_000_000_000_i64)
        .with_display_name("Shown".to_string())
        .with_type("gear".to_string())
        .with_source_url("s3://w".to_string())
        .await
        .unwrap();
    assert_eq!(created.created_at, Some(1_700_000_000_000));
    assert_eq!(created.r#type.as_deref(), Some("gear"));
    let body = seen
        .all()
        .into_iter()
        .find_map(|e| e.strip_prefix("body ").map(str::to_string))
        .unwrap();
    let body: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(body["created_at"], json!(1_700_000_000_000_i64));
    // Explicit JSON name; `emit_unpopulated` field.
    assert_eq!(body["sourceUrl"], json!("s3://w"));
    assert_eq!(body["tags"], json!([]));
    assert_eq!(body["display_name"], json!("Shown"));

    // Models write the same JSON outside the client.
    let json = serde_json::to_value(CreateWidgetRequest {
        name: "x".into(),
        created_at: Some(5),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(json["created_at"], json!(5));
    let widget = serde_json::to_value(Widget {
        name: "x".into(),
        size_bytes: 7,
        metadata_location: "m".into(),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(widget["size_bytes"], json!(7));
    assert_eq!(widget["metadata-location"], json!("m"));
}

#[tokio::test]
async fn error_answers_keep_status_code_message_and_details() {
    let (base, _) = serve().await;
    let client = DemoClient::new_unauthenticated(base);

    let Error::Api(e) = client.widget("missing").get().await.unwrap_err() else {
        panic!("expected an API error");
    };
    assert_eq!(e.status, 404);
    assert_eq!(e.code.as_deref(), Some("WIDGET_NOT_FOUND"));
    assert_eq!(e.error_code(), Some(ErrorCode::WIDGET_NOT_FOUND));
    assert_eq!(e.message, "no widget named missing");
    assert_eq!(e.details, vec![json!({"reason": "gone"})]);
    assert_eq!(e.to_string(), "WIDGET_NOT_FOUND: no widget named missing");
    assert!(
        e.body.starts_with('{') && e.body.contains("\"reason\":\"gone\""),
        "{}",
        e.body
    );

    // An unknown code, under the camelCase key.
    let err = client.widget("odd").get().await.unwrap_err();
    assert_eq!(err.status(), Some(400));
    assert_eq!(err.code(), Some("SOMETHING_NEW"));
    let Error::Api(e) = err else { unreachable!() };
    assert_eq!(e.error_code(), None);

    // A non-JSON body.
    let err = client.widget("text").get().await.unwrap_err();
    assert_eq!(err.status(), Some(502));
    assert_eq!(err.code(), None);
    assert_eq!(err.to_string(), "HTTP 502: bad gateway");
    let Error::Api(e) = err else { unreachable!() };
    assert_eq!(e.body, "bad gateway\n");

    // An `error` envelope, and OAuth's keys.
    let err = client.widget("nested").get().await.unwrap_err();
    assert_eq!(err.code(), Some("AlreadyExistsException"));
    assert_eq!(err.to_string(), "AlreadyExistsException: table exists");
    let err = client.widget("oauth").get().await.unwrap_err();
    assert_eq!(err.code(), Some("invalid_grant"));
    assert_eq!(err.to_string(), "invalid_grant: token expired");
}

#[tokio::test]
async fn list_stream_reads_all_pages() {
    let (base, _) = serve().await;
    let client = DemoClient::new_unauthenticated(base);
    let page = client.list_widgets().await.unwrap();
    assert_eq!(page.widgets.len(), 2);
    let all: Vec<Widget> = client
        .list_widgets()
        .into_stream()
        .try_collect()
        .await
        .unwrap();
    let names: Vec<_> = all.iter().map(|w| w.name.as_str()).collect();
    assert_eq!(names, ["w1", "w2", "w3"]);
}

#[tokio::test]
async fn list_stream_holds_its_cap_when_pages_are_larger() {
    let (base, seen) = serve().await;
    let client = DemoClient::new_unauthenticated(base);
    // The server's first page always has 2 widgets.
    let one: Vec<Widget> = client
        .list_widgets()
        .with_max_results(1)
        .into_stream()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(one.len(), 1);
    let three: Vec<Widget> = client
        .list_widgets()
        .with_max_results(3)
        .into_stream()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(three.len(), 3);
    let queries: Vec<_> = seen
        .all()
        .into_iter()
        .filter(|e| e.starts_with("query "))
        .collect();
    assert_eq!(
        queries,
        [
            "query max_results=1",
            "query max_results=3",
            "query max_results=1&page_token=p2"
        ]
    );
}

#[tokio::test]
async fn free_form_json_keeps_whole_numbers() {
    use buffa_types::google::protobuf::{Struct, Value};
    let (base, seen) = serve().await;
    let client = DemoClient::new_unauthenticated(base);
    let mut nested = Struct::default();
    nested.fields.insert("id".into(), Value::from(7_i64));
    let mut attributes = Struct::default();
    attributes.fields.insert("count".into(), Value::from(3_i64));
    attributes.fields.insert("ratio".into(), Value::from(0.5));
    attributes
        .fields
        .insert("nested".into(), Value::from(nested));
    client
        .create_widget("w")
        .with_attributes(attributes)
        .await
        .unwrap();
    let body = seen
        .all()
        .into_iter()
        .find_map(|e| e.strip_prefix("body ").map(str::to_string))
        .unwrap();
    for part in [r#""count":3"#, r#""id":7"#, r#""ratio":0.5"#] {
        assert!(body.contains(part), "{part} in {body}");
    }

    let widget = client.widget("json").get().await.unwrap();
    let attributes = widget.attributes.as_option().unwrap();
    assert_eq!(attributes.fields["count"], Value::from(3_i64));
    assert_eq!(widget.notes.len(), 2);
    assert_eq!(
        widget.rows["t"].values,
        [Value::from(1_i64), Value::from(2_i64)]
    );
    let json = serde_json::to_string(&widget).unwrap();
    assert!(json.contains(r#""rows":{"t":[1,2]}"#), "{json}");

    // Repeated and map `int64` values.
    let widget = Widget {
        sizes: vec![1, 2],
        counts: [("a".to_string(), 3)].into_iter().collect(),
        ..Default::default()
    };
    let json = serde_json::to_string(&widget).unwrap();
    assert!(json.contains(r#""sizes":[1,2]"#), "{json}");
    assert!(json.contains(r#""counts":{"a":3}"#), "{json}");
    let back: Widget = serde_json::from_str(r#"{"sizes":[1,"2"],"counts":{"a":"3"}}"#).unwrap();
    assert_eq!((back.sizes, back.counts["a"]), (vec![1, 2], 3));
}

#[tokio::test]
async fn head_requests_and_get_bodies() {
    let (base, seen) = serve().await;
    let client = DemoClient::new_unauthenticated(base);
    assert!(client.widget_exists("w").await.unwrap());
    assert!(!client.widget_exists("missing").await.unwrap());
    let err = client.widget_exists("denied").await.unwrap_err();
    assert_eq!(err.status(), Some(403));
    let found = client
        .search_widgets(
            "w",
            vec!["a".to_string()],
            [(
                "t".to_string(),
                buffa_types::google::protobuf::ListValue {
                    values: vec![1_i64.into(), 2_i64.into()],
                    ..Default::default()
                },
            )],
        )
        .await
        .unwrap();
    let names: Vec<_> = found.widgets.iter().map(|w| w.name.as_str()).collect();
    assert_eq!(names, ["w1", "w2"]);
    let seen = seen.all();
    assert!(seen.contains(&"HEAD w".to_string()), "{seen:?}");
    assert!(
        seen.contains(&r#"GET body {"prefix":"w","tags":["a"],"filters":{"t":[1,2]}}"#.to_string()),
        "{seen:?}"
    );
}

#[tokio::test]
async fn path_values_stay_out_of_the_body() {
    let (base, seen) = serve().await;
    let client = DemoClient::new_unauthenticated(base);
    client.tag_widget("w", "blue").await.unwrap();
    // `RenameWidget` is in `path_fields_in_body_methods`.
    client.rename_widget("w", "v").await.unwrap();
    let seen = seen.all();
    assert!(
        seen.contains(&r#"tag w body {"tag":"blue"}"#.to_string()),
        "{seen:?}"
    );
    assert!(
        seen.contains(&r#"rename w body {"name":"w","new_name":"v"}"#.to_string()),
        "{seen:?}"
    );
}

#[tokio::test]
async fn ndjson_answers_and_header_fields() {
    let (base, _seen) = serve().await;
    let client = DemoClient::new_unauthenticated(base);
    let widgets = client
        .stream_widgets()
        .with_prefix("w".to_string())
        .await
        .unwrap();
    let names: Vec<_> = widgets.iter().map(|w| w.name.as_str()).collect();
    assert_eq!(names, ["w1", "w2"]);
    assert_eq!(widgets[0].created_at, 1);

    let version = client.widget_version("w").await.unwrap();
    assert_eq!(version.name, "w");
    assert_eq!(version.version, 7);
    assert_eq!(version.etag.as_deref(), Some("abc"));
    let bare = client.widget_version("bare").await.unwrap();
    assert_eq!((bare.name.as_str(), bare.version), ("", 7));
}

#[tokio::test]
async fn raw_bodies_and_paths_of_several_segments() {
    let (base, seen) = serve().await;
    let client = DemoClient::new_unauthenticated(base);
    // A multi-segment path value keeps its `/` and drops the leading one.
    client
        .put_blob("/a/b c.bin", vec![0, 1, 255])
        .await
        .unwrap();
    assert_eq!(
        seen.all(),
        ["PUT /api/v1/blobs/a/b%20c.bin application/octet-stream [0, 1, 255]"]
    );
    let blob = client.get_blob("x/y").await.unwrap();
    assert_eq!(blob.data, b"blob:x/y");
    assert_eq!(blob.size, Some(8));
    let part = client
        .get_blob("x/y")
        .with_range("bytes=0-1".to_string())
        .await
        .unwrap();
    assert_eq!(part.data, b"range:bytes=0-1");
}

#[tokio::test]
async fn query_keys_are_explicit_json_names() {
    let (base, seen) = serve().await;
    let client = DemoClient::new_unauthenticated(base);
    client.list_widgets().with_first(3).await.unwrap();
    client
        .list_widgets()
        .with_order_by("name".to_string())
        .await
        .unwrap();
    let seen = seen.all();
    assert!(seen.contains(&"query startIndex=3".to_string()), "{seen:?}");
    assert!(seen.contains(&"query orderBy=name".to_string()), "{seen:?}");
}

#[tokio::test]
async fn bearer_token_goes_in_every_request_and_form_bodies_are_encoded() {
    let (base, seen) = serve().await;
    let client = DemoClient::new_with_token(base, "secret");
    client.widget("w").delete().await.unwrap();
    let token = client
        .exchange_token(
            "urn:ietf:params:oauth:grant-type:token-exchange",
            "id-token",
        )
        .await
        .unwrap();
    assert_eq!(token.access_token, "issued");
    assert_eq!(token.expires_in, 3600);
    let seen = seen.all();
    assert_eq!(
        seen.iter().filter(|e| *e == "auth Bearer secret").count(),
        2,
        "{seen:?}"
    );
    assert!(seen.contains(&"content-type application/x-www-form-urlencoded".to_string()));
    assert!(seen.contains(
        &"body grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Atoken-exchange&subject_token=id-token"
            .to_string()
    ));

    #[cfg(not(feature = "cloud"))]
    {
        let transport = crate::codegen::transport::HttpClient::default().with_token("secret");
        assert!(!format!("{transport:?}").contains("secret"));
    }
}

#[cfg(not(feature = "cloud"))]
#[tokio::test]
async fn user_agent_goes_in_every_request() {
    let (base, seen) = serve().await;
    let default = format!("user-agent demo-client/{}", env!("CARGO_PKG_VERSION"));
    DemoClient::new_unauthenticated(base.clone())
        .widget("w")
        .delete()
        .await
        .unwrap();
    DemoClient::new_with_token(base.clone(), "secret")
        .widget("w")
        .delete()
        .await
        .unwrap();
    DemoClient::new_unauthenticated(base)
        .with_user_agent("demo-cli/1.0")
        .widget("w")
        .delete()
        .await
        .unwrap();
    let agents: Vec<String> = seen
        .all()
        .into_iter()
        .filter(|e| e.starts_with("user-agent "))
        .collect();
    assert_eq!(
        agents,
        [default.clone(), default, "user-agent demo-cli/1.0".into()]
    );
}

/// The `User-Agent` of `CloudClient` comes from `client_options()`, for every constructor.
#[cfg(feature = "cloud")]
#[tokio::test]
async fn cloud_user_agent_goes_in_every_request() {
    let (base, seen) = serve().await;
    let default = format!("user-agent demo-client/{}", env!("CARGO_PKG_VERSION"));
    let bearer = olai_http::bearer::BearerAuth::static_token("secret");
    let clients = [
        DemoClient::new_unauthenticated(base.clone()),
        DemoClient::new_with_token(base.clone(), "secret"),
        DemoClient::new_with_bearer(base.clone(), bearer).unwrap(),
    ];
    for client in clients {
        client.widget("w").delete().await.unwrap();
    }
    let agents: Vec<String> = seen
        .all()
        .into_iter()
        .filter(|e| e.starts_with("user-agent "))
        .collect();
    assert_eq!(agents, [default.clone(), default.clone(), default]);
}

/// Signs with a bearer token once it has one, and sends requests without one until then,
/// as a signer that asks the server what it takes.
#[cfg(feature = "cloud")]
#[derive(Debug, Default)]
struct Late(std::sync::Mutex<Option<String>>);

#[cfg(feature = "cloud")]
impl olai_http::RequestSigner for Late {
    fn sign<'a>(
        &'a self,
        request: olai_http::reqwest::RequestBuilder,
    ) -> futures::future::BoxFuture<'a, olai_http::Result<olai_http::reqwest::RequestBuilder>> {
        let token = self.0.lock().unwrap().clone();
        Box::pin(async move {
            Ok(match token {
                Some(token) => request.bearer_auth(token),
                None => request,
            })
        })
    }
}

#[cfg(feature = "cloud")]
#[tokio::test]
async fn cloud_client_takes_bearer_providers_and_signers() {
    let (base, seen) = serve().await;
    let provider = olai_http::bearer::BearerAuth::static_token("provided");
    let client = DemoClient::new_with_bearer(base.clone(), provider).unwrap();
    client.widget("w").delete().await.unwrap();
    assert!(seen.all().contains(&"auth Bearer provided".to_string()));

    let (base, seen) = serve().await;
    let late = Arc::new(Late::default());
    let client = DemoClient::new_with_signer(base, late.clone()).unwrap();
    client.widget("w").delete().await.unwrap();
    assert!(
        !seen.all().iter().any(|e| e.starts_with("auth ")),
        "{:?}",
        seen.all()
    );
    *late.0.lock().unwrap() = Some("late".into());
    client.widget("w").delete().await.unwrap();
    assert!(seen.all().contains(&"auth Bearer late".to_string()));
}
