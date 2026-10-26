//! Git 연동. 상태·브랜치·스테이지·커밋·푸시는 git 명령(CLI)을 부르고, 에디터 거터의 변경 표시는
//! 인덱스(스테이지된) 내용과 버퍼를 줄 단위로 견준다(저장하지 않은 편집도 바로 보인다).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use similar::{DiffOp, TextDiff};

use crate::text_file;

/// 거터 변경 표시를 계산할 최대 크기. 넘으면 표시하지 않는다(큰 파일에서 입력이 느려지지 않게).
pub const MAX_DIFF_BYTES: usize = 4 * 1024 * 1024;
/// 줄 비교가 이보다 오래 걸리면 근사 결과로 끝낸다.
const DIFF_TIMEOUT: Duration = Duration::from_millis(50);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repo {
    root: PathBuf,
}

/// 파일 하나의 변경 종류.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Modified,
    Added,
    Deleted,
    Renamed,
    /// 아직 git이 모르는 파일.
    Untracked,
    /// 병합 충돌.
    Conflicted,
}

impl Change {
    /// 목록에 보일 한 글자.
    pub fn letter(self) -> char {
        match self {
            Change::Modified => 'M',
            Change::Added => 'A',
            Change::Deleted => 'D',
            Change::Renamed => 'R',
            Change::Untracked => 'U',
            Change::Conflicted => '!',
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// 저장소 루트 기준 상대 경로(`/` 구분).
    pub path: String,
    /// 스테이지된 변경(인덱스 ↔ HEAD).
    pub staged: Option<Change>,
    /// 스테이지하지 않은 변경(작업 폴더 ↔ 인덱스).
    pub unstaged: Option<Change>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    /// 현재 브랜치. 분리된 HEAD면 None.
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub entries: Vec<Entry>,
}

/// 거터에 보일 줄의 변경.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineChange {
    Added,
    Modified,
    /// 이 줄 바로 앞에서 줄이 지워졌다.
    DeletedAbove,
}

impl Repo {
    /// `path`(파일이나 폴더)를 담은 저장소. git이 없거나 저장소가 아니면 None.
    pub fn discover(path: &Path) -> Option<Repo> {
        let dir = if path.is_dir() { path } else { path.parent()? };
        let output = run(dir, &["rev-parse", "--show-toplevel"], None).ok()?;
        let root = String::from_utf8(output).ok()?;
        let root = PathBuf::from(root.trim());
        root.is_dir().then_some(Repo { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn git(&self, args: &[&str]) -> Result<Vec<u8>, String> {
        run(&self.root, args, None)
    }

    /// 저장소 기준 상대 경로(`/` 구분). 저장소 밖이면 None.
    pub fn relative(&self, path: &Path) -> Option<String> {
        let strip = |path: &Path, root: &Path| {
            let relative = path.strip_prefix(root).ok()?;
            Some(relative.to_string_lossy().replace('\\', "/"))
        };
        strip(&dunce(path), &dunce(&self.root)).or_else(|| {
            // git은 실제 경로를 돌려준다. macOS의 /var → /private/var 같은 링크나 Windows의 짧은 이름
            // (RUNNER~1)을 거친 경로는 실제 경로로 바꿔 다시 견준다.
            let real = dunce(&path.canonicalize().ok()?);
            strip(&real, &dunce(&self.root.canonicalize().ok()?))
        })
    }

    pub fn status(&self) -> Result<Status, String> {
        let output = self.git(&[
            "--no-optional-locks",
            "status",
            "--porcelain=v2",
            "--branch",
            "-z",
            "--untracked-files=all",
        ])?;
        Ok(parse_status(&String::from_utf8_lossy(&output)))
    }

    /// 인덱스(스테이지된) 내용. 인덱스에 없는 파일(새 파일)이면 None.
    pub fn indexed(&self, path: &Path) -> Option<Vec<u8>> {
        let relative = self.relative(path)?;
        self.git(&["show", &format!(":{relative}")]).ok()
    }

    /// 파일 하나의 통합 diff. `staged`면 스테이지된 변경, 아니면 작업 폴더의 변경. 새 파일은 전체를 추가로 보인다.
    pub fn diff(&self, entry: &Entry, staged: bool) -> Result<String, String> {
        let output = if !staged && entry.unstaged == Some(Change::Untracked) {
            // 새 파일은 비교할 원본이 없어 --no-index로 빈 파일과 견준다(차이가 있으면 종료 코드가 1이다).
            let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
            run_allow(
                &self.root,
                &["diff", "--no-color", "--no-index", "--", null, &entry.path],
                &[0, 1],
            )?
        } else {
            let mut args = vec!["diff", "--no-color"];
            if staged {
                args.push("--cached");
            }
            args.extend(["--", &entry.path]);
            self.git(&args)?
        };
        Ok(text_file::decode(&output).map_or_else(
            |_| String::from_utf8_lossy(&output).into_owned(),
            |content| content.text,
        ))
    }

    pub fn stage(&self, paths: &[String]) -> Result<(), String> {
        let mut args = vec!["add", "--"];
        args.extend(paths.iter().map(String::as_str));
        self.git(&args).map(drop)
    }

    pub fn unstage(&self, paths: &[String]) -> Result<(), String> {
        let mut args = vec!["restore", "--staged", "--"];
        args.extend(paths.iter().map(String::as_str));
        self.git(&args).map(drop)
    }

    /// 스테이지된 변경을 커밋한다. 메시지는 표준 입력으로 넘긴다(따옴표·줄바꿈을 그대로).
    pub fn commit(&self, message: &str) -> Result<String, String> {
        if message.trim().is_empty() {
            return Err("커밋 메시지를 넣으세요".into());
        }
        let output = run(&self.root, &["commit", "-F", "-"], Some(message.as_bytes()))?;
        Ok(String::from_utf8_lossy(&output)
            .lines()
            .next()
            .unwrap_or("")
            .to_owned())
    }

    /// 올린다. 추적 브랜치가 없으면 origin에 같은 이름으로 만들어 추적한다. 오래 걸리므로 UI 스레드가
    /// 아닌 곳에서 부른다. 인증은 git의 자격 증명 도우미에 맡긴다(터미널 질문은 막는다).
    pub fn push(&self, status: &Status) -> Result<String, String> {
        let output = match (&status.upstream, &status.branch) {
            (Some(_), _) => self.git(&["push"])?,
            (None, Some(branch)) => self.git(&["push", "--set-upstream", "origin", branch])?,
            (None, None) => return Err("브랜치에 있지 않습니다(분리된 HEAD)".into()),
        };
        Ok(String::from_utf8_lossy(&output).trim().to_owned())
    }

    pub fn branches(&self) -> Result<Vec<String>, String> {
        let output = self.git(&["for-each-ref", "--format=%(refname:short)", "refs/heads"])?;
        Ok(String::from_utf8_lossy(&output)
            .lines()
            .map(str::to_owned)
            .filter(|b| !b.is_empty())
            .collect())
    }

    /// 브랜치를 바꾼다. 커밋하지 않은 변경과 겹치면 git이 거부하고 그 이유를 돌려준다.
    pub fn switch(&self, branch: &str) -> Result<(), String> {
        self.git(&["switch", "--", branch]).map(drop)
    }
}

/// Windows의 `\\?\` 접두사를 떼어 경로를 견줄 수 있게 한다(git은 접두사 없는 경로를 돌려준다).
fn dunce(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC\\") => PathBuf::from(rest),
        _ => path.to_path_buf(),
    }
}

fn run(dir: &Path, args: &[&str], stdin: Option<&[u8]>) -> Result<Vec<u8>, String> {
    run_with(dir, args, stdin, &[0])
}

fn run_allow(dir: &Path, args: &[&str], ok: &[i32]) -> Result<Vec<u8>, String> {
    run_with(dir, args, None, ok)
}

fn run_with(
    dir: &Path,
    args: &[&str],
    stdin: Option<&[u8]>,
    ok: &[i32],
) -> Result<Vec<u8>, String> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(dir)
        // 한글 경로를 \354\202\254 같은 이스케이프 없이 받는다.
        .args(["-c", "core.quotepath=false"])
        .args(args)
        // 자격 증명을 터미널에서 묻다가 멈추지 않게 한다(GUI 자격 증명 도우미는 그대로 쓴다).
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // GUI 앱에서 부를 때 콘솔 창이 깜박이지 않게 한다.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command.spawn().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => {
            "git을 찾을 수 없습니다(PATH에 git을 설치하세요)".to_owned()
        }
        _ => format!("git 실행 실패: {e}"),
    })?;
    if let (Some(input), Some(mut pipe)) = (stdin, child.stdin.take()) {
        pipe.write_all(input)
            .map_err(|e| format!("git 입력 실패: {e}"))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|e| format!("git 실행 실패: {e}"))?;
    if output.status.code().is_some_and(|code| ok.contains(&code)) {
        Ok(output.stdout)
    } else {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        Err(if message.is_empty() {
            format!("git {} 실패({})", args.join(" "), output.status)
        } else {
            message
        })
    }
}

/// `git status --porcelain=v2 --branch -z` 출력.
fn parse_status(text: &str) -> Status {
    let mut status = Status::default();
    let mut records = text.split('\0').filter(|r| !r.is_empty());
    while let Some(record) = records.next() {
        if let Some(header) = record.strip_prefix("# ") {
            let (key, value) = header.split_once(' ').unwrap_or((header, ""));
            match key {
                "branch.head" if value != "(detached)" => status.branch = Some(value.to_owned()),
                "branch.upstream" => status.upstream = Some(value.to_owned()),
                "branch.ab" => {
                    for part in value.split(' ') {
                        if let Some(n) = part.strip_prefix('+') {
                            status.ahead = n.parse().unwrap_or(0);
                        } else if let Some(n) = part.strip_prefix('-') {
                            status.behind = n.parse().unwrap_or(0);
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        let kind = record.as_bytes()[0];
        // 필드 수: 1(보통) 8개 뒤 경로, 2(이름 바꿈) 9개 뒤 경로 + 다음 레코드에 원래 경로, u(충돌) 10개 뒤 경로.
        let (fields, path_at) = match kind {
            b'1' => (record.splitn(9, ' ').collect::<Vec<_>>(), 8),
            b'2' => (record.splitn(10, ' ').collect::<Vec<_>>(), 9),
            b'u' => (record.splitn(11, ' ').collect::<Vec<_>>(), 10),
            b'?' => {
                status.entries.push(Entry {
                    path: record[2..].to_owned(),
                    staged: None,
                    unstaged: Some(Change::Untracked),
                });
                continue;
            }
            _ => continue,
        };
        if kind == b'2' {
            records.next(); // 원래 경로
        }
        let Some(path) = fields.get(path_at) else {
            continue;
        };
        let xy = fields.get(1).map(|s| s.as_bytes()).unwrap_or(b"..");
        let (staged, unstaged) = if kind == b'u' {
            (None, Some(Change::Conflicted))
        } else {
            (code(xy[0]), code(xy[1]))
        };
        status.entries.push(Entry {
            path: (*path).to_owned(),
            staged,
            unstaged,
        });
    }
    status
}

fn code(c: u8) -> Option<Change> {
    match c {
        b'M' | b'T' => Some(Change::Modified),
        b'A' | b'C' => Some(Change::Added),
        b'D' => Some(Change::Deleted),
        b'R' => Some(Change::Renamed),
        b'U' => Some(Change::Conflicted),
        _ => None,
    }
}

/// `base`(인덱스 내용) 대비 `current`(버퍼)의 줄 변경. (현재 줄 번호, 변경) 목록, 줄 순.
pub fn line_changes(base: &str, current: &str) -> Vec<(usize, LineChange)> {
    if base.len() > MAX_DIFF_BYTES || current.len() > MAX_DIFF_BYTES {
        return Vec::new();
    }
    // 마지막 줄의 줄바꿈 유무로 그 줄이 바뀐 것으로 보이지 않게 둘 다 줄바꿈으로 끝낸다.
    let terminated = |text: &str| {
        if text.is_empty() || text.ends_with('\n') {
            text.to_owned()
        } else {
            format!("{text}\n")
        }
    };
    let (base, current) = (terminated(base), terminated(current));
    let diff = TextDiff::configure()
        .timeout(DIFF_TIMEOUT)
        .diff_lines(&base, &current);
    let lines = current.lines().count();
    let mut changes = Vec::new();
    for op in diff.ops() {
        match *op {
            DiffOp::Equal { .. } => {}
            DiffOp::Insert {
                new_index, new_len, ..
            } => changes.extend((new_index..new_index + new_len).map(|l| (l, LineChange::Added))),
            DiffOp::Replace {
                new_index, new_len, ..
            } => {
                changes.extend((new_index..new_index + new_len).map(|l| (l, LineChange::Modified)))
            }
            DiffOp::Delete { new_index, .. } => {
                // 끝에서 지운 줄은 마지막 줄에 표시한다.
                changes.push((
                    new_index.min(lines.saturating_sub(1)),
                    LineChange::DeletedAbove,
                ));
            }
        }
    }
    changes.sort_by_key(|(line, _)| *line);
    changes.dedup_by_key(|(line, _)| *line);
    changes
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn 줄_변경은_추가_수정_삭제를_구분한다() {
        let base = "a\nb\nc\nd\n";
        assert_eq!(line_changes(base, base), []);
        assert_eq!(
            line_changes(base, "a\nb\nX\nc\nd\n"),
            [(2, LineChange::Added)]
        );
        assert_eq!(
            line_changes(base, "a\nB\nc\nd\n"),
            [(1, LineChange::Modified)]
        );
        assert_eq!(
            line_changes(base, "a\nc\nd\n"),
            [(1, LineChange::DeletedAbove)]
        );
        assert_eq!(
            line_changes(base, "a\nb\nc\n"),
            [(2, LineChange::DeletedAbove)],
            "끝에서 지움"
        );
        assert_eq!(line_changes("a\nb", "a\nb\n"), [], "마지막 줄바꿈만 다름");
        assert_eq!(line_changes("", "x\n"), [(0, LineChange::Added)]);
    }

    #[test]
    fn 상태는_스테이지_여부와_이름_바꿈_충돌_새_파일을_읽는다() {
        let text = "# branch.oid abc\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +2 -1\0\
1 .M N... 100644 100644 100644 a a src/파일 이름.rs\0\
1 A. N... 000000 100644 100644 0 b new.txt\0\
2 R. N... 100644 100644 100644 a a R100 renamed.txt\0old.txt\0\
u UU N... 100644 100644 100644 100644 a b c conflict.txt\0\
? untracked.txt\0";
        let status = parse_status(text);
        assert_eq!(status.branch.as_deref(), Some("main"));
        assert_eq!(status.upstream.as_deref(), Some("origin/main"));
        assert_eq!((status.ahead, status.behind), (2, 1));
        let entry = |path: &str| {
            status
                .entries
                .iter()
                .find(|e| e.path == path)
                .unwrap()
                .clone()
        };
        assert_eq!(
            entry("src/파일 이름.rs").unstaged,
            Some(Change::Modified),
            "공백·한글 경로"
        );
        assert_eq!(entry("new.txt").staged, Some(Change::Added));
        assert_eq!(entry("renamed.txt").staged, Some(Change::Renamed));
        assert_eq!(entry("conflict.txt").unstaged, Some(Change::Conflicted));
        assert_eq!(entry("untracked.txt").unstaged, Some(Change::Untracked));
        assert_eq!(
            status.entries.len(),
            5,
            "이름 바꿈의 원래 경로는 따로 세지 않는다"
        );
        assert_eq!(parse_status("# branch.head (detached)\0").branch, None);
    }

    fn git(dir: &Path, args: &[&str]) {
        run(dir, args, None).unwrap();
    }

    #[test]
    fn 실제_저장소에서_스테이지_커밋_브랜치를_다룬다() {
        let dir = std::env::temp_dir().join(format!("ferrite-git-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q", "-b", "main"]);
        git(&dir, &["config", "user.name", "Ferrite Test"]);
        git(&dir, &["config", "user.email", "test@example.invalid"]);
        git(&dir, &["config", "commit.gpgsign", "false"]);
        fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
        let repo = Repo::discover(&dir.join("a.txt")).expect("저장소");
        assert_eq!(repo.relative(&dir.join("a.txt")).as_deref(), Some("a.txt"));

        let status = repo.status().unwrap();
        assert_eq!(status.branch.as_deref(), Some("main"));
        assert_eq!(status.entries[0].unstaged, Some(Change::Untracked));
        assert!(repo.indexed(&dir.join("a.txt")).is_none(), "인덱스에 없다");
        assert!(
            repo.diff(&status.entries[0], false)
                .unwrap()
                .contains("+one")
        );

        repo.stage(&["a.txt".into()]).unwrap();
        assert_eq!(
            repo.status().unwrap().entries[0].staged,
            Some(Change::Added)
        );
        let err = repo.commit("  ").unwrap_err();
        assert!(err.contains("메시지"), "{err}");
        repo.commit("첫 커밋 \"따옴표\"\n\n본문").unwrap();
        assert!(repo.status().unwrap().entries.is_empty());
        assert_eq!(repo.indexed(&dir.join("a.txt")).unwrap(), b"one\ntwo\n");

        fs::write(dir.join("a.txt"), "one\nTWO\n").unwrap();
        let entry = repo.status().unwrap().entries[0].clone();
        assert_eq!(entry.unstaged, Some(Change::Modified));
        let diff = repo.diff(&entry, false).unwrap();
        assert!(diff.contains("-two") && diff.contains("+TWO"), "{diff}");
        repo.stage(&["a.txt".into()]).unwrap();
        repo.unstage(&["a.txt".into()]).unwrap();
        assert_eq!(repo.status().unwrap().entries[0].staged, None);

        git(&dir, &["branch", "work"]);
        assert_eq!(repo.branches().unwrap(), ["main", "work"]);
        repo.switch("work").unwrap();
        assert_eq!(repo.status().unwrap().branch.as_deref(), Some("work"));
        assert!(repo.switch("없는-브랜치").is_err());

        let err = repo.push(&repo.status().unwrap()).unwrap_err();
        assert!(!err.is_empty(), "origin이 없으면 git의 이유를 돌려준다");

        // 추적 브랜치가 없으면 origin에 같은 이름으로 만들고 추적한다.
        let remote = dir.with_extension("remote.git");
        let _ = fs::remove_dir_all(&remote);
        git(&dir, &["init", "-q", "--bare", &remote.to_string_lossy()]);
        git(
            &dir,
            &["remote", "add", "origin", &remote.to_string_lossy()],
        );
        repo.push(&repo.status().unwrap()).unwrap();
        let status = repo.status().unwrap();
        assert_eq!(status.upstream.as_deref(), Some("origin/work"));
        assert_eq!((status.ahead, status.behind), (0, 0));
        fs::remove_dir_all(&remote).unwrap();
        assert!(Repo::discover(&std::env::temp_dir().join("ferrite-git-none-xyz")).is_none());
        fs::remove_dir_all(&dir).unwrap();
    }
}
