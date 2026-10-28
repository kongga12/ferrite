//! WASM 플러그인. 플러그인 폴더(`plugins/<이름>/`)마다 `plugin.toml`과 `.wasm`이 있다. 플러그인은
//! 명령과 패널을 매니페스트에 적고, 앱은 부를 때마다 요청 JSON을 플러그인 메모리에 써 넣고 답 JSON을
//! 읽는다.
//!
//! 플러그인이 내보내는 것(wasm32 코어 모듈):
//! - `memory`
//! - `ferrite_alloc(len: i32) -> i32`: 요청을 쓸 `len`바이트 자리. 플러그인이 `ferrite_call`에서 푼다.
//! - `ferrite_call(ptr: i32, len: i32) -> i64`: 요청(UTF-8 JSON)을 받아 답의 위치(위 32비트)와
//!   길이(아래 32비트)를 돌려준다. 답은 다음 호출까지 그대로 둔다.
//!
//! 앱은 아무것도 주지 않으므로(가져오기 없음) 플러그인은 파일·네트워크·시계를 못 쓰고 요청에 담긴
//! 것만 본다. 호출마다 연료(명령 수)와 메모리에 상한을 둔다. 넘기거나 멈추면(trap) 상태가 깨졌을 수
//! 있으므로 인스턴스를 버리고 다음 호출에 새로 띄운다.

use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wasmi::{
    Config, Engine, Linker, Memory, Module, Store, StoreLimits, StoreLimitsBuilder, TrapCode,
    TypedFunc,
};

/// 앱이 아는 플러그인 API 판(매니페스트의 `api`).
pub const API_VERSION: u32 = 1;
pub const MANIFEST: &str = "plugin.toml";
/// 호출 하나의 연료. 릴리스 빌드에서 끝없는 반복이 0.33초에 다 쓰고, 예제의 4MiB 글자 세기는
/// 0.1초에 끝난다(측정). UI 스레드에서 부르므로 이것이 멈춤의 상한이다.
// ponytail: UI 스레드에서 부른다. 무거운 플러그인이 생기면 작업 스레드로 옮긴다.
const FUEL: u64 = 1_000_000_000;
const MAX_MEMORY: usize = 256 * 1024 * 1024;
const MAX_REPLY: usize = 16 * 1024 * 1024;
/// 이보다 큰 문서는 글(`text`)을 넘기지 않는다(null).
pub const MAX_TEXT: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize)]
pub struct Manifest {
    pub api: u32,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    /// 플러그인 폴더 안의 .wasm 파일 이름.
    #[serde(default = "default_wasm")]
    pub wasm: String,
    #[serde(default)]
    pub commands: Vec<Contribution>,
    #[serde(default)]
    pub panels: Vec<Contribution>,
}

fn default_wasm() -> String {
    "plugin.wasm".into()
}

/// 플러그인이 더하는 명령이나 패널.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Contribution {
    pub id: String,
    pub title: String,
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Self, String> {
        let manifest: Manifest = toml::from_str(text).map_err(|e| format!("{MANIFEST}: {e}"))?;
        if manifest.api != API_VERSION {
            return Err(format!(
                "지원하지 않는 플러그인 API 판입니다(api = {}, Ferrite는 {API_VERSION})",
                manifest.api
            ));
        }
        if manifest.id.trim().is_empty() {
            return Err(format!("{MANIFEST}: id가 비었습니다"));
        }
        // 플러그인 폴더 밖의 파일을 가리키지 못하게 한다.
        let wasm = Path::new(&manifest.wasm);
        if manifest.wasm.is_empty() || !wasm.components().all(|c| matches!(c, Component::Normal(_)))
        {
            return Err(format!(
                "{MANIFEST}: wasm은 플러그인 폴더 안의 파일 이름이어야 합니다: {}",
                manifest.wasm
            ));
        }
        Ok(manifest)
    }
}

/// 명령·패널을 부를 때 넘기는 활성 에디터.
pub struct Editor<'a> {
    pub path: &'a Path,
    pub language: Option<&'a str>,
    pub text: &'a str,
    pub selection: &'a str,
}

impl Editor<'_> {
    fn to_json(&self) -> Value {
        json!({
            "path": self.path.to_string_lossy(),
            "language": self.language,
            "text": (self.text.len() <= MAX_TEXT).then_some(self.text),
            "selection": self.selection,
        })
    }
}

/// 패널에서 일어난 일.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Click { id: String },
    Change { id: String, value: String },
}

/// 플러그인의 답. 명령은 `actions`만, 패널은 `ui`도 쓴다.
#[derive(Debug, Default, Deserialize, PartialEq)]
pub struct Reply {
    #[serde(default)]
    pub actions: Vec<Action>,
    #[serde(default)]
    pub ui: Vec<Widget>,
}

/// 플러그인이 앱에 시키는 일. 모르는 종류는 버린다(새 판 플러그인이 옛 앱에서도 돈다).
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    /// 활성 에디터의 선택 영역(없으면 커서 자리)을 바꾼다.
    ReplaceSelection { text: String },
    /// 상태 표시줄에 알린다.
    Message { text: String },
    /// 이 플러그인의 패널을 플러그인 창에 연다.
    ShowPanel { panel: String },
    #[serde(other)]
    Unknown,
}

/// 패널에 그릴 것. 모르는 종류는 그리지 않는다.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Widget {
    Heading {
        text: String,
    },
    Label {
        text: String,
    },
    /// 고정폭 글.
    Code {
        text: String,
    },
    Separator,
    Button {
        id: String,
        text: String,
    },
    /// 한 줄 입력. 고칠 때마다 `change` 이벤트가 가므로, 플러그인은 받은 값을 `value`로 돌려줘야 한다.
    Input {
        id: String,
        #[serde(default)]
        value: String,
        #[serde(default)]
        hint: String,
    },
    #[serde(other)]
    Unknown,
}

pub struct Plugin {
    pub manifest: Manifest,
    pub dir: PathBuf,
    engine: Engine,
    module: Module,
    running: Option<Running>,
    /// 호출 하나의 연료(테스트는 줄인다).
    fuel: u64,
}

struct Running {
    store: Store<StoreLimits>,
    memory: Memory,
    alloc: TypedFunc<i32, i32>,
    call: TypedFunc<(i32, i32), i64>,
}

impl Plugin {
    /// 플러그인 폴더를 읽고 한 번 띄워 내보낸 것을 확인한다.
    pub fn load(dir: &Path) -> Result<Self, String> {
        let text =
            fs::read_to_string(dir.join(MANIFEST)).map_err(|e| format!("{MANIFEST}: {e}"))?;
        let manifest = Manifest::parse(&text)?;
        let bytes =
            fs::read(dir.join(&manifest.wasm)).map_err(|e| format!("{}: {e}", manifest.wasm))?;
        let mut config = Config::default();
        config.consume_fuel(true);
        let engine = Engine::new(&config);
        let module = Module::new(&engine, &bytes).map_err(|e| format!("{}: {e}", manifest.wasm))?;
        let mut plugin = Self {
            manifest,
            dir: dir.to_path_buf(),
            engine,
            module,
            running: None,
            fuel: FUEL,
        };
        plugin.running = Some(plugin.start()?);
        Ok(plugin)
    }

    fn start(&self) -> Result<Running, String> {
        let limits = StoreLimitsBuilder::new()
            .memory_size(MAX_MEMORY)
            .instances(1)
            .build();
        let mut store = Store::new(&self.engine, limits);
        store.limiter(|limits| limits);
        // 시작 함수(start)도 연료를 쓴다.
        store.set_fuel(self.fuel).map_err(|e| e.to_string())?;
        let instance = Linker::new(&self.engine)
            .instantiate_and_start(&mut store, &self.module)
            .map_err(|e| format!("플러그인을 띄우지 못했습니다: {e}"))?;
        let memory = instance
            .get_memory(&store, "memory")
            .ok_or("플러그인이 memory를 내보내지 않습니다")?;
        let alloc = instance
            .get_typed_func(&store, "ferrite_alloc")
            .map_err(|e| format!("ferrite_alloc(i32) -> i32: {e}"))?;
        let call = instance
            .get_typed_func(&store, "ferrite_call")
            .map_err(|e| format!("ferrite_call(i32, i32) -> i64: {e}"))?;
        Ok(Running {
            store,
            memory,
            alloc,
            call,
        })
    }

    pub fn command(&mut self, id: &str, editor: Option<&Editor>) -> Result<Reply, String> {
        self.call(&json!({
            "type": "command",
            "command": id,
            "editor": editor.map(Editor::to_json),
        }))
    }

    /// 패널을 그린다. `event`가 없으면 처음 열거나 에디터가 바뀌어 다시 그리는 것이다.
    pub fn panel(
        &mut self,
        id: &str,
        event: Option<&Event>,
        editor: Option<&Editor>,
    ) -> Result<Reply, String> {
        self.call(&json!({
            "type": "panel",
            "panel": id,
            "event": event,
            "editor": editor.map(Editor::to_json),
        }))
    }

    fn call(&mut self, request: &Value) -> Result<Reply, String> {
        let request = serde_json::to_vec(request).map_err(|e| e.to_string())?;
        let reply = self.exchange(&request);
        if reply.is_err() {
            self.running = None;
        }
        serde_json::from_slice(&reply?).map_err(|e| format!("플러그인의 답을 읽지 못했습니다: {e}"))
    }

    fn exchange(&mut self, request: &[u8]) -> Result<Vec<u8>, String> {
        if self.running.is_none() {
            self.running = Some(self.start()?);
        }
        let Some(r) = self.running.as_mut() else {
            unreachable!("바로 위에서 띄웠다");
        };
        r.store.set_fuel(self.fuel).map_err(|e| e.to_string())?;
        let len = i32::try_from(request.len()).map_err(|_| "요청이 너무 큽니다")?;
        let ptr = r.alloc.call(&mut r.store, len).map_err(trap)?;
        r.memory
            .write(&mut r.store, ptr as u32 as usize, request)
            .map_err(|_| "플러그인이 준 요청 자리가 메모리 밖입니다")?;
        let packed = r.call.call(&mut r.store, (ptr, len)).map_err(trap)? as u64;
        let (ptr, len) = ((packed >> 32) as usize, (packed & 0xffff_ffff) as usize);
        if len > MAX_REPLY {
            return Err(format!("플러그인의 답이 너무 큽니다({len}바이트)"));
        }
        let mut reply = vec![0; len];
        r.memory
            .read(&r.store, ptr, &mut reply)
            .map_err(|_| "플러그인의 답이 메모리 밖에 있습니다")?;
        Ok(reply)
    }
}

fn trap(e: wasmi::Error) -> String {
    if e.as_trap_code() == Some(TrapCode::OutOfFuel) {
        "플러그인이 너무 오래 걸려 멈췄습니다".into()
    } else {
        format!("플러그인 오류: {e}")
    }
}

/// `root`의 하위 폴더 중 `plugin.toml`이 있는 것을 이름 순으로 읽는다. 읽지 못한 플러그인은 오류로
/// 돌려주고 나머지는 계속 읽는다. `root`가 없으면 빈 목록이다.
pub fn load_all(root: &Path) -> (Vec<Plugin>, Vec<String>) {
    let mut plugins: Vec<Plugin> = Vec::new();
    let mut errors = Vec::new();
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == ErrorKind::NotFound => return (plugins, errors),
        Err(e) => return (plugins, vec![format!("{}: {e}", root.display())]),
    };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.join(MANIFEST).is_file())
        .collect();
    dirs.sort();
    for dir in dirs {
        match Plugin::load(&dir) {
            Ok(p) if plugins.iter().any(|q| q.manifest.id == p.manifest.id) => {
                errors.push(format!(
                    "{}: 같은 id({})의 플러그인이 이미 있습니다",
                    dir.display(),
                    p.manifest.id
                ))
            }
            Ok(p) => plugins.push(p),
            Err(e) => errors.push(format!("{}: {e}", dir.display())),
        }
    }
    (plugins, errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST_TEXT: &str = "api = 1\nid = \"t\"\nname = \"시험\"\nwasm = \"t.wat\"\n";

    /// `ferrite_call`의 본문을 받아 시험 플러그인을 만든다. 메모리 1024번지에 `reply`를 둔다.
    fn plugin(name: &str, call_body: &str, reply: &str) -> Result<Plugin, String> {
        let dir =
            std::env::temp_dir().join(format!("ferrite-plugin-{name}-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(MANIFEST), MANIFEST_TEXT).unwrap();
        let reply = reply.replace('\\', "\\\\").replace('"', "\\\"");
        fs::write(
            dir.join("t.wat"),
            format!(
                r#"(module
                  (memory (export "memory") 2)
                  (data (i32.const 1024) "{reply}")
                  (func (export "ferrite_alloc") (param i32) (result i32) (i32.const 4096))
                  (func (export "ferrite_call") (param i32 i32) (result i64) {call_body}))"#
            ),
        )
        .unwrap();
        let plugin = Plugin::load(&dir);
        fs::remove_dir_all(&dir).unwrap();
        plugin
    }

    /// 1024번지에서 `len`바이트를 답한다.
    fn reply_at_1024(len: usize) -> String {
        format!("(i64.or (i64.shl (i64.const 1024) (i64.const 32)) (i64.const {len}))")
    }

    #[test]
    fn 매니페스트의_기본값과_잘못된_값() {
        let m = Manifest::parse("api = 1\nid = \"a\"\nname = \"가\"\n").unwrap();
        assert_eq!((m.wasm.as_str(), m.commands.len()), ("plugin.wasm", 0));
        let m = Manifest::parse(
            "api = 1\nid = \"a\"\nname = \"가\"\n[[commands]]\nid = \"c\"\ntitle = \"명령\"\n",
        )
        .unwrap();
        assert_eq!(
            m.commands,
            [Contribution {
                id: "c".into(),
                title: "명령".into()
            }]
        );
        assert!(
            Manifest::parse("api = 2\nid = \"a\"\nname = \"가\"\n")
                .unwrap_err()
                .contains("api = 2")
        );
        assert!(Manifest::parse("api = 1\nid = \" \"\nname = \"가\"\n").is_err());
        for wasm in ["../x.wasm", "/x.wasm", "a/../../x.wasm", ""] {
            let text = format!("api = 1\nid = \"a\"\nname = \"가\"\nwasm = \"{wasm}\"\n");
            assert!(Manifest::parse(&text).is_err(), "{wasm}");
        }
    }

    #[test]
    fn 답을_읽고_모르는_종류는_버린다() {
        let reply = r#"{"actions":[{"type":"message","text":"안녕"},{"type":"beep"}],"ui":[{"type":"separator"},{"type":"input","id":"w"},{"type":"chart"}]}"#;
        let mut p = plugin("ok", &reply_at_1024(reply.len()), reply).unwrap();
        let editor = Editor {
            path: Path::new("a.txt"),
            language: None,
            text: "abc",
            selection: "",
        };
        let got = p
            .panel("p", Some(&Event::Click { id: "b".into() }), Some(&editor))
            .unwrap();
        assert_eq!(
            got.actions,
            [
                Action::Message {
                    text: "안녕".into()
                },
                Action::Unknown
            ]
        );
        assert_eq!(
            got.ui,
            [
                Widget::Separator,
                Widget::Input {
                    id: "w".into(),
                    value: String::new(),
                    hint: String::new()
                },
                Widget::Unknown
            ]
        );
    }

    #[test]
    fn 끝나지_않는_플러그인은_연료가_떨어져_멈추고_다시_불러도_된다() {
        let mut p = plugin("loop", "(loop (br 0)) (i64.const 0)", "").unwrap();
        // 디버그 빌드의 인터프리터는 느려서 연료를 줄인다.
        p.fuel = 1_000_000;
        for _ in 0..2 {
            assert_eq!(
                p.command("c", None).unwrap_err(),
                "플러그인이 너무 오래 걸려 멈췄습니다"
            );
        }
    }

    #[test]
    fn 잘못된_답은_오류다() {
        let mut p = plugin("trap", "unreachable", "").unwrap();
        assert!(
            p.command("c", None)
                .unwrap_err()
                .starts_with("플러그인 오류")
        );
        // 메모리(2쪽 = 128KiB) 밖
        let mut p = plugin(
            "oob",
            "(i64.or (i64.shl (i64.const 200000) (i64.const 32)) (i64.const 10))",
            "",
        )
        .unwrap();
        assert_eq!(
            p.command("c", None).unwrap_err(),
            "플러그인의 답이 메모리 밖에 있습니다"
        );
        let mut p = plugin("big", "(i64.const 0xffffffff)", "").unwrap();
        assert!(p.command("c", None).unwrap_err().contains("너무 큽니다"));
        let mut p = plugin("json", &reply_at_1024(3), "{x}").unwrap();
        assert!(
            p.command("c", None)
                .unwrap_err()
                .starts_with("플러그인의 답을 읽지 못했습니다")
        );
    }

    #[test]
    fn 앱이_주지_않는_것을_가져오거나_내보내지_않으면_불러오지_못한다() {
        let dir = std::env::temp_dir().join(format!("ferrite-plugin-bad-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(MANIFEST), MANIFEST_TEXT).unwrap();
        fs::write(dir.join("t.wat"), r#"(module (import "env" "now" (func)))"#).unwrap();
        assert!(
            Plugin::load(&dir)
                .err()
                .unwrap()
                .starts_with("플러그인을 띄우지 못했습니다")
        );
        fs::write(
            dir.join("t.wat"),
            r#"(module (memory (export "memory") 1))"#,
        )
        .unwrap();
        assert!(
            Plugin::load(&dir)
                .err()
                .unwrap()
                .starts_with("ferrite_alloc")
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn 폴더의_플러그인을_이름_순으로_읽고_실패는_따로_모은다() {
        let root = std::env::temp_dir().join(format!("ferrite-plugins-{}", std::process::id()));
        let wat = r#"(module (memory (export "memory") 1)
            (func (export "ferrite_alloc") (param i32) (result i32) (i32.const 0))
            (func (export "ferrite_call") (param i32 i32) (result i64) (i64.const 0)))"#;
        for (dir, id) in [("b", "same"), ("a", "same"), ("c", "other")] {
            fs::create_dir_all(root.join(dir)).unwrap();
            let text = format!("api = 1\nid = \"{id}\"\nname = \"{dir}\"\nwasm = \"t.wat\"\n");
            fs::write(root.join(dir).join(MANIFEST), text).unwrap();
            fs::write(root.join(dir).join("t.wat"), wat).unwrap();
        }
        fs::create_dir_all(root.join("d")).unwrap();
        fs::write(root.join("d").join(MANIFEST), "api = 1\n").unwrap();
        fs::create_dir_all(root.join("no-manifest")).unwrap();
        let (plugins, errors) = load_all(&root);
        let names: Vec<_> = plugins.iter().map(|p| p.manifest.name.as_str()).collect();
        assert_eq!(names, ["a", "c"]);
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors[0].contains("같은 id(same)"));
        assert!(errors[1].contains(MANIFEST));
        fs::remove_dir_all(&root).unwrap();
        assert!(load_all(&root).0.is_empty());
    }
}
