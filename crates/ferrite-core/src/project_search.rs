//! 프로젝트 전체 검색: 파일 이름 퍼지 검색(파일로 이동)과 전체 텍스트 검색(파일에서 찾기).
//! 둘 다 `.gitignore`·`.ignore`를 따르는 같은 파일 목록을 쓴다(git 저장소가 아니어도 따른다).

use std::cmp::Reverse;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use ignore::overrides::OverrideBuilder;
use ignore::{WalkBuilder, WalkState};
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use regex::{Regex, RegexBuilder};

use crate::text_file;

/// `.gitignore`가 없어도 늘 빼는 폴더(버전 관리, 빌드 결과물, 내려받은 패키지).
const SKIPPED_DIRS: [&str; 5] = [".git", ".svn", ".hg", "node_modules", "target"];
/// 전체 텍스트 검색에서 이보다 큰 파일은 읽지 않는다(로그·덤프).
const MAX_SEARCH_FILE: u64 = 20 * 1024 * 1024;
/// 결과 줄을 보여 줄 때 자르는 길이(글자).
const MAX_LINE_CHARS: usize = 240;
/// 첫 일치 앞에 남길 글자 수(긴 줄을 자를 때).
const CONTEXT_CHARS: usize = 40;
/// 전체 텍스트 검색의 일치 수 상한. 넘으면 멈추고 잘렸다고 알린다.
pub const MAX_TEXT_MATCHES: usize = 20_000;

fn walker(root: &Path, globs: &[String]) -> Result<WalkBuilder, String> {
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(false)
        .require_git(false)
        .filter_entry(|entry| {
            let dir = entry.file_type().is_some_and(|t| t.is_dir());
            !(dir
                && entry
                    .file_name()
                    .to_str()
                    .is_some_and(|n| SKIPPED_DIRS.contains(&n)))
        });
    if !globs.is_empty() {
        let mut overrides = OverrideBuilder::new(root);
        for glob in globs {
            overrides
                .add(glob)
                .map_err(|e| format!("파일 패턴 {glob}: {e}"))?;
        }
        builder.overrides(overrides.build().map_err(|e| e.to_string())?);
    }
    Ok(builder)
}

fn relative(root: &Path, path: &Path) -> Option<String> {
    Some(
        path.strip_prefix(root)
            .ok()?
            .to_string_lossy()
            .replace('\\', "/"),
    )
}

/// 프로젝트의 파일들(루트 기준 상대 경로, `/` 구분), 이름순. `cancel`이 서면 그때까지 찾은 것.
pub fn list_files(root: &Path, cancel: &AtomicBool) -> Vec<String> {
    let found = Mutex::new(Vec::new());
    let Ok(builder) = walker(root, &[]) else {
        return Vec::new();
    };
    builder.build_parallel().run(|| {
        let found = &found;
        Box::new(move |entry| {
            if cancel.load(Ordering::Relaxed) {
                return WalkState::Quit;
            }
            if let Ok(entry) = entry
                && entry.file_type().is_some_and(|t| t.is_file())
                && let Some(path) = relative(root, entry.path())
            {
                found.lock().unwrap_or_else(|e| e.into_inner()).push(path);
            }
            WalkState::Continue
        })
    });
    let mut files = found.into_inner().unwrap_or_else(|e| e.into_inner());
    files.sort();
    files
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileMatch {
    /// `files`의 번호.
    pub index: usize,
    pub score: u32,
    /// 일치한 글자의 위치(글자 단위, 오름차순).
    pub positions: Vec<u32>,
}

/// `files`에서 `query`에 맞는 것을 점수 높은 순으로 최대 `limit`개. 점수가 같으면 짧은 경로가 먼저다.
/// 검색어가 비면 짧은 경로부터. 대소문자는 검색어에 대문자가 있을 때만 가린다.
pub fn fuzzy_files(files: &[String], query: &str, limit: usize) -> Vec<FileMatch> {
    let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
    let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
    let mut buf = Vec::new();
    let mut scored: Vec<(u32, usize)> = files
        .iter()
        .enumerate()
        .filter_map(|(i, file)| {
            pattern
                .score(Utf32Str::new(file, &mut buf), &mut matcher)
                .map(|score| (score, i))
        })
        .collect();
    let key = |&(score, i): &(u32, usize)| (Reverse(score), files[i].len(), i);
    if scored.len() > limit {
        scored.select_nth_unstable_by_key(limit, key);
        scored.truncate(limit);
    }
    scored.sort_unstable_by_key(key);
    scored
        .into_iter()
        .map(|(score, index)| {
            let mut positions = Vec::new();
            pattern.indices(
                Utf32Str::new(&files[index], &mut buf),
                &mut matcher,
                &mut positions,
            );
            positions.sort_unstable();
            positions.dedup();
            FileMatch {
                index,
                score,
                positions,
            }
        })
        .collect()
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextQuery {
    pub pattern: String,
    pub regex: bool,
    pub case_sensitive: bool,
    pub whole_word: bool,
    /// 찾을 파일 패턴. 쉼표로 나누고 `!`로 시작하면 뺀다(예: `*.java, *.xml, !*Test.java`).
    pub files: String,
}

impl TextQuery {
    pub fn compile(&self) -> Result<Regex, String> {
        if self.pattern.is_empty() {
            return Err("찾을 내용을 넣으세요".into());
        }
        let mut pattern = if self.regex {
            self.pattern.clone()
        } else {
            regex::escape(&self.pattern)
        };
        if self.whole_word {
            pattern = format!(r"\b(?:{pattern})\b");
        }
        RegexBuilder::new(&pattern)
            .case_insensitive(!self.case_sensitive)
            .multi_line(true)
            .build()
            .map_err(|e| e.to_string())
    }

    fn globs(&self) -> Vec<String> {
        self.files
            .split(',')
            .map(str::trim)
            .filter(|g| !g.is_empty())
            .map(str::to_owned)
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineMatch {
    /// 0부터 센 줄 번호.
    pub line: usize,
    /// 첫 일치의 줄 안 바이트 위치(이동할 때 쓴다).
    pub column: usize,
    /// 보여 줄 줄(앞 공백을 빼고 길면 자른다).
    pub text: String,
    /// `text` 안의 일치 범위(바이트).
    pub ranges: Vec<Range<usize>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileHits {
    pub path: PathBuf,
    pub lines: Vec<LineMatch>,
}

impl FileHits {
    pub fn count(&self) -> usize {
        self.lines.iter().map(|l| l.ranges.len()).sum()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextStats {
    /// 읽은 파일 수.
    pub files: usize,
    pub matches: usize,
    /// 상한(`MAX_TEXT_MATCHES`)에 걸려 멈췄다.
    pub truncated: bool,
}

/// 프로젝트 파일에서 찾는다. 찾은 파일마다 `found`를 부른다(여러 스레드에서 불린다).
/// 파일은 UTF-8·CP949로 읽고, 바이너리·너무 큰 파일은 건너뛴다. `cancel`이 서면 멈춘다.
pub fn search_text(
    root: &Path,
    query: &TextQuery,
    cancel: &AtomicBool,
    found: &(dyn Fn(FileHits) + Sync),
) -> Result<TextStats, String> {
    let re = query.compile()?;
    let builder = walker(root, &query.globs())?;
    let files = AtomicUsize::new(0);
    let matches = AtomicUsize::new(0);
    let truncated = AtomicBool::new(false);
    builder.build_parallel().run(|| {
        let (re, files, matches, truncated) = (&re, &files, &matches, &truncated);
        Box::new(move |entry| {
            if cancel.load(Ordering::Relaxed) || truncated.load(Ordering::Relaxed) {
                return WalkState::Quit;
            }
            let Ok(entry) = entry else {
                return WalkState::Continue;
            };
            if !entry.file_type().is_some_and(|t| t.is_file())
                || entry.metadata().is_ok_and(|m| m.len() > MAX_SEARCH_FILE)
            {
                return WalkState::Continue;
            }
            let Some(text) = std::fs::read(entry.path())
                .ok()
                .and_then(|bytes| text_file::decode(&bytes).ok())
                .map(|content| content.text)
            else {
                return WalkState::Continue;
            };
            files.fetch_add(1, Ordering::Relaxed);
            let lines = match_lines(re, &text);
            if lines.is_empty() {
                return WalkState::Continue;
            }
            let hits = FileHits {
                path: entry.path().to_path_buf(),
                lines,
            };
            let total = matches.fetch_add(hits.count(), Ordering::Relaxed) + hits.count();
            found(hits);
            if total >= MAX_TEXT_MATCHES {
                truncated.store(true, Ordering::Relaxed);
                return WalkState::Quit;
            }
            WalkState::Continue
        })
    });
    Ok(TextStats {
        files: files.into_inner(),
        matches: matches.into_inner(),
        truncated: truncated.into_inner(),
    })
}

/// 일치를 줄마다 모은다. 여러 줄에 걸친 일치는 첫 줄까지만 칠한다.
fn match_lines(re: &Regex, text: &str) -> Vec<LineMatch> {
    let mut lines: Vec<(usize, usize, Vec<Range<usize>>)> = Vec::new();
    let (mut line, mut line_start, mut scanned) = (0, 0, 0);
    for m in re.find_iter(text).filter(|m| !m.is_empty()) {
        for (i, _) in text[scanned..m.start()].match_indices('\n') {
            line += 1;
            line_start = scanned + i + 1;
        }
        scanned = m.start();
        let line_end = text[line_start..]
            .find('\n')
            .map_or(text.len(), |i| line_start + i);
        let range = m.start() - line_start..m.end().min(line_end) - line_start;
        match lines.last_mut() {
            Some((n, _, ranges)) if *n == line => ranges.push(range),
            _ => lines.push((line, line_start, vec![range])),
        }
    }
    lines
        .into_iter()
        .map(|(line, start, ranges)| {
            let end = text[start..].find('\n').map_or(text.len(), |i| start + i);
            let column = ranges[0].start;
            let (text, ranges) = snippet(&text[start..end], ranges);
            LineMatch {
                line,
                column,
                text,
                ranges,
            }
        })
        .collect()
}

/// 앞 공백을 빼고, 길면 첫 일치가 보이게 앞뒤를 잘라 `…`을 붙인다. 범위도 그만큼 옮긴다.
fn snippet(line: &str, ranges: Vec<Range<usize>>) -> (String, Vec<Range<usize>>) {
    let line = line.trim_end_matches('\r');
    let mut start = line.len() - line.trim_start().len();
    let first = ranges[0].start.max(start);
    // 첫 일치 앞이 길면 CONTEXT_CHARS 글자만 남긴다.
    let before: Vec<usize> = line[start..first]
        .char_indices()
        .map(|(i, _)| start + i)
        .collect();
    let mut prefix = "";
    if before.len() > CONTEXT_CHARS {
        start = before[before.len() - CONTEXT_CHARS];
        prefix = "…";
    }
    let mut end = line.len();
    let mut suffix = "";
    if let Some((i, _)) = line[start..].char_indices().nth(MAX_LINE_CHARS) {
        end = start + i;
        suffix = "…";
    }
    let shift = prefix.len();
    let ranges = ranges
        .into_iter()
        .filter(|r| r.start >= start && r.start < end)
        .map(|r| r.start - start + shift..r.end.min(end) - start + shift)
        .collect();
    (format!("{prefix}{}{suffix}", &line[start..end]), ranges)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ferrite-search-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(root: &Path, path: &str, bytes: &[u8]) {
        let file = root.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, bytes).unwrap();
    }

    #[test]
    fn 파일_목록은_gitignore와_빌드_폴더를_뺀다() {
        let root = temp("list");
        write(&root, ".gitignore", b"*.log\nbuild/\n");
        write(&root, "src/main/java/A.java", b"");
        write(&root, ".github/workflows/ci.yml", b"");
        write(&root, "app.log", b"");
        write(&root, "build/out.class", b"");
        write(&root, "target/classes/A.class", b"");
        write(&root, ".git/config", b"");
        write(&root, "node_modules/x/index.js", b"");
        let files = list_files(&root, &AtomicBool::new(false));
        assert_eq!(
            files,
            [
                ".github/workflows/ci.yml",
                ".gitignore",
                "src/main/java/A.java"
            ],
            "git 저장소가 아니어도 .gitignore를 따르고, 숨김 파일은 넣는다"
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn 퍼지_검색은_경로_경계와_짧은_경로를_앞에_둔다() {
        let files: Vec<String> = [
            "src/main/java/egovframework/example/sample/web/EgovSampleController.java",
            "src/main/java/egovframework/example/sample/service/SampleVO.java",
            "src/main/resources/egovframework/sqlmap/example/mappers/EgovSample_SQL.xml",
            "docs/sample.md",
            "README.md",
        ]
        .map(str::to_owned)
        .to_vec();
        let found = fuzzy_files(&files, "samplecontroller", 10);
        assert_eq!(found[0].index, 0);
        assert_eq!(found.len(), 1);

        let found = fuzzy_files(&files, "sample", 2);
        assert_eq!(found.len(), 2);
        assert_eq!(
            files[found[0].index], "docs/sample.md",
            "같은 점수면 짧은 경로"
        );

        // 일치 위치는 글자 단위다(한글 경로도).
        let korean = vec!["문서/설계서.md".to_owned()];
        let found = fuzzy_files(&korean, "설계", 5);
        assert_eq!(found[0].positions, [3, 4]);

        assert_eq!(fuzzy_files(&files, "", 3).len(), 3);
        assert!(fuzzy_files(&files, "zzz", 3).is_empty());
    }

    #[test]
    fn 텍스트_검색은_cp949도_읽고_파일_패턴을_따른다() {
        let root = temp("text");
        write(
            &root,
            "a/Sample.java",
            "class Sample {\n    // 사용자 조회\n    String name; String names;\n}\n".as_bytes(),
        );
        let (cp949, _, _) = encoding_rs::EUC_KR.encode("<p>사용자 목록</p>\r\n");
        write(&root, "b/list.jsp", &cp949);
        write(&root, "c/data.bin", "\0\0\0사용자".as_bytes());
        let collect = |query: &TextQuery| {
            let hits = Mutex::new(Vec::new());
            let stats = search_text(&root, query, &AtomicBool::new(false), &|h| {
                hits.lock().unwrap().push(h)
            })
            .unwrap();
            let mut hits = hits.into_inner().unwrap();
            hits.sort_by(|a, b| a.path.cmp(&b.path));
            (hits, stats)
        };

        let (hits, stats) = collect(&TextQuery {
            pattern: "사용자".into(),
            ..Default::default()
        });
        assert_eq!(stats.matches, 2);
        assert_eq!(hits.len(), 2, "바이너리 파일은 건너뛴다");
        assert_eq!(hits[0].lines[0].line, 1);
        assert_eq!(hits[0].lines[0].text, "// 사용자 조회");
        assert_eq!(
            &hits[0].lines[0].text[hits[0].lines[0].ranges[0].clone()],
            "사용자"
        );
        assert_eq!(hits[0].lines[0].column, 7, "원래 줄의 바이트 위치");
        assert_eq!(hits[1].lines[0].text, "<p>사용자 목록</p>", "CP949·CRLF");

        let (hits, _) = collect(&TextQuery {
            pattern: "name".into(),
            whole_word: true,
            ..Default::default()
        });
        assert_eq!(hits[0].lines[0].ranges.len(), 1, "names는 단어가 다르다");

        let (hits, _) = collect(&TextQuery {
            pattern: "사용자".into(),
            files: "*.jsp".into(),
            ..Default::default()
        });
        assert_eq!(hits.len(), 1);
        assert!(hits[0].path.ends_with("b/list.jsp"));

        let (hits, _) = collect(&TextQuery {
            pattern: "SAMPLE".into(),
            case_sensitive: true,
            ..Default::default()
        });
        assert!(hits.is_empty());
        let err = search_text(
            &root,
            &TextQuery {
                pattern: "(".into(),
                regex: true,
                ..Default::default()
            },
            &AtomicBool::new(false),
            &|_| {},
        )
        .unwrap_err();
        assert!(!err.is_empty());
        fs::remove_dir_all(&root).unwrap();
    }

    /// Phase 3 완료 조건: 10만 파일에서 퍼지 검색 결과가 100ms 이내.
    /// `cargo test --release -p ferrite-core -- --ignored --nocapture 퍼지`
    #[test]
    #[ignore = "릴리스 빌드로 잰다"]
    fn 퍼지_검색은_10만_파일에서_100ms_안에_끝난다() {
        let words = [
            "board", "member", "notice", "auth", "code", "file", "menu", "log", "stat", "user",
            "dept", "approval", "schedule", "survey", "banner", "popup", "mail", "sms",
        ];
        let layers = [
            ("src/main/java/egovframework/com", "web", "Controller.java"),
            (
                "src/main/java/egovframework/com",
                "service/impl",
                "ServiceImpl.java",
            ),
            ("src/main/java/egovframework/com", "service", "VO.java"),
            (
                "src/main/resources/egovframework/mapper/com",
                "",
                "_SQL_oracle.xml",
            ),
            (
                "src/main/webapp/WEB-INF/jsp/egovframework/com",
                "",
                "List.jsp",
            ),
        ];
        let mut files = Vec::new();
        let mut n = 0u64;
        while files.len() < 100_000 {
            // 결정적인 의사 난수로 모듈·기능 이름을 섞는다.
            n = n
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let pick = |shift: u32| words[(n >> shift) as usize % words.len()];
            let (base, layer, suffix) = layers[(n >> 7) as usize % layers.len()];
            let name = format!("Egov{}{}{}", pick(13), pick(23), files.len());
            files.push(format!(
                "{base}/{}/{}/{layer}/{name}{suffix}",
                pick(33),
                pick(43)
            ));
        }
        for query in [
            "noticecontroller",
            "egovboardsql",
            "mbrlist.jsp",
            "s",
            "zzzz",
        ] {
            let started = std::time::Instant::now();
            let found = fuzzy_files(&files, query, 50);
            let took = started.elapsed();
            eprintln!(
                "{query}: {}개, {:.1}ms",
                found.len(),
                took.as_secs_f64() * 1000.0
            );
            assert!(took.as_millis() < 100, "{query}: {took:?}");
        }
    }

    #[test]
    fn 긴_줄은_첫_일치가_보이게_자른다() {
        let line = format!("{}needle{}", "가".repeat(100), "나".repeat(400));
        let (text, ranges) = snippet(
            &line,
            vec![Range {
                start: 300,
                end: 306,
            }],
        );
        assert!(text.starts_with('…') && text.ends_with('…'));
        assert_eq!(&text[ranges[0].clone()], "needle");
        assert_eq!(text.chars().count(), MAX_LINE_CHARS + 2);
    }
}
