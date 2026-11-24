//! A module process that seals values through the secret server's key service: the consumer
//! the live key test (`tests/live_keys.rs`) runs in a container.
//!
//! `key_probe serve` connects one `KeyServiceProvider` (socket `TRESTLE_SDS_SOCKET`, key name
//! `TRESTLE_KEY_PROBE_KEK`, both set by the planner) and keeps it for its whole life, behind an
//! olai-store `ManagedObjectStore` over an in-memory backend. It takes commands on a control
//! socket, `/tmp/key-probe.sock`, one line each, and answers one line: `ok <result>` or
//! `err <reason>`. `key_probe ctl <command…>` sends one (the test runs it with `docker exec`):
//!
//! - `active`: the KEK id new values are sealed under, as the provider last saw it.
//! - `seal <name> <value>`: create object `<name>` with the sensitive field `value`.
//! - `open <name>`: the object's `value`, unsealed.
//! - `seal-raw <tag>` / `open-raw <tag>`: seal a fixed value as a bare envelope kept in memory
//!   (out of the store, so no sweep moves it), and open it again.
//! - `usage`: the store's blobs per KEK id, as JSON.
//! - `rewrap`: run `rewrap_all`; answers `<rewrapped> <failed>`.
//!
//! The values are test data: it never handles key material, only the DEKs the server unwraps.
//!
//! `cargo run -p olai-stack-runtime --example key_probe -- serve`

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use olai_key_service::KeyServiceProvider;
use olai_store::{
    EnvelopeEncryptor, FieldRole, InMemoryStore, Label, ManagedObjectStore, Object, ObjectStore,
    ResourceFieldDescriptor, ResourceName, ResourceRegistry, ResourceTypeDescriptor, RewrapOptions,
    SecretObjectReader,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Mutex;

const CONTROL: &str = "/tmp/key-probe.sock";
const RAW_VALUE: &[u8] = b"a raw envelope";

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
struct Secret;

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("secret")
    }
}

impl FromStr for Secret {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        (s == "secret")
            .then_some(Secret)
            .ok_or_else(|| s.to_owned())
    }
}

impl Label for Secret {
    fn as_str(&self) -> &str {
        "secret"
    }
}

static FIELDS: &[ResourceFieldDescriptor] = &[ResourceFieldDescriptor {
    name: "value",
    role: FieldRole::Sensitive,
}];

static TYPES: &[ResourceTypeDescriptor<Secret>] = &[ResourceTypeDescriptor {
    label: Secret,
    fields: FIELDS,
    path_names: &["name"],
    parent_label: None,
}];

struct Probe {
    encryptor: EnvelopeEncryptor,
    store: ManagedObjectStore<Secret, InMemoryStore<Secret>>,
    objects: Mutex<BTreeMap<String, Object<Secret>>>,
    raw: Mutex<BTreeMap<String, Vec<u8>>>,
}

impl Probe {
    async fn run(&self, line: &str) -> Result<String, String> {
        let words: Vec<&str> = line.split_whitespace().collect();
        match words.as_slice() {
            ["active"] => Ok(self.encryptor.active_kek_id().to_string()),
            ["seal", name, value] => {
                let object = self
                    .store
                    .create(
                        Secret,
                        &ResourceName::from_naive_str_split(name),
                        Some(serde_json::json!({ "value": value })),
                        None,
                        None,
                    )
                    .await
                    .map_err(|e| e.to_string())?;
                self.objects.lock().await.insert(name.to_string(), object);
                Ok(name.to_string())
            }
            ["open", name] => {
                let id = self
                    .objects
                    .lock()
                    .await
                    .get(*name)
                    .map(|o| o.id)
                    .ok_or("no such object")?;
                let object = self
                    .store
                    .get_with_secrets(&id)
                    .await
                    .map_err(|e| e.to_string())?;
                object
                    .properties
                    .as_ref()
                    .and_then(|p| p["value"].as_str())
                    .map(str::to_owned)
                    .ok_or_else(|| "no value".to_owned())
            }
            ["seal-raw", tag] => {
                let blob = self
                    .encryptor
                    .seal(tag, RAW_VALUE)
                    .await
                    .map_err(|e| e.to_string())?;
                let kek = olai_store::envelope_kek_id(&blob).map_err(|e| e.to_string())?;
                self.raw.lock().await.insert(tag.to_string(), blob);
                Ok(kek.to_string())
            }
            ["open-raw", tag] => {
                let blob = self
                    .raw
                    .lock()
                    .await
                    .get(*tag)
                    .cloned()
                    .ok_or("no such envelope")?;
                let value = self
                    .encryptor
                    .open(tag, &blob)
                    .await
                    .map_err(|e| e.to_string())?;
                (value == RAW_VALUE)
                    .then(|| "opened".to_owned())
                    .ok_or_else(|| "opened to another value".to_owned())
            }
            ["usage"] => {
                let usage = self.store.kek_usage().await.map_err(|e| e.to_string())?;
                Ok(serde_json::json!({
                    "by_kek": usage.by_kek.iter().map(|(k, n)| (k.to_string(), n)).collect::<BTreeMap<_, _>>(),
                    "unreadable": usage.unreadable,
                })
                .to_string())
            }
            ["rewrap"] => {
                let report = self
                    .store
                    .rewrap_all(RewrapOptions::default())
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(format!("{} {}", report.rewrapped, report.failed.len()))
            }
            _ => Err(format!("unknown command: {line}")),
        }
    }
}

async fn serve() {
    let socket = std::env::var("TRESTLE_SDS_SOCKET").expect("TRESTLE_SDS_SOCKET");
    let key = std::env::var("TRESTLE_KEY_PROBE_KEK").expect("TRESTLE_KEY_PROBE_KEK");
    let provider = KeyServiceProvider::connect(&socket, &key)
        .await
        .expect("the key service answers");
    println!("key-probe sealing under {key} from {socket}");
    let encryptor = EnvelopeEncryptor::new(Arc::new(provider));
    let probe = Arc::new(Probe {
        store: ManagedObjectStore::with_encryptor(
            InMemoryStore::new(),
            encryptor.clone(),
            ResourceRegistry::from_static(TYPES),
        ),
        encryptor,
        objects: Mutex::default(),
        raw: Mutex::default(),
    });
    let _ = std::fs::remove_file(CONTROL);
    let listener = UnixListener::bind(CONTROL).expect("the control socket binds");
    loop {
        let (stream, _) = listener.accept().await.expect("accept");
        let probe = Arc::clone(&probe);
        tokio::spawn(async move {
            let (read, mut write) = stream.into_split();
            let mut line = String::new();
            if BufReader::new(read).read_line(&mut line).await.is_err() {
                return;
            }
            let answer = match probe.run(line.trim()).await {
                Ok(result) => format!("ok {result}\n"),
                Err(reason) => format!("err {}\n", reason.replace('\n', " ")),
            };
            let _ = write.write_all(answer.as_bytes()).await;
        });
    }
}

async fn ctl(command: &str) -> i32 {
    let mut stream = UnixStream::connect(CONTROL)
        .await
        .expect("the probe is serving");
    stream
        .write_all(format!("{command}\n").as_bytes())
        .await
        .expect("write");
    let mut answer = String::new();
    BufReader::new(stream)
        .read_line(&mut answer)
        .await
        .expect("read");
    print!("{answer}");
    i32::from(!answer.starts_with("ok "))
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("serve") => serve().await,
        Some("ctl") => std::process::exit(ctl(&args[1..].join(" ")).await),
        _ => {
            eprintln!("usage: key_probe serve | key_probe ctl <command…>");
            std::process::exit(2);
        }
    }
}
