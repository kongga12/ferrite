//! 생태계별 버전 비교. 숫자 조각은 수로, 글자 조각은 생태계의 사전 배포 순서로 견준다.
//!
//! - Maven(ComparableVersion): alpha < beta < milestone < rc < snapshot < 정식(ga·final·release) < sp,
//!   그 밖의 글자는 sp 뒤에서 사전 순.
//! - PyPI(PEP 440): dev < a < b < rc < 정식 < post.
//! - npm·crates.io(semver): `-` 뒤 사전 배포는 정식보다 앞이고, `+` 뒤 빌드 정보는 견주지 않는다.
//!
//! 숫자 조각은 글자 조각보다 뒤다(`1.0.1` > `1.0-rc1`). 짧은 쪽의 빈 자리는 0 또는 정식으로 본다.
// ponytail: 생태계 규칙의 공통 부분만 따른다(Maven의 `-`와 `.` 구분, PEP 440 epoch·local은 보지 않는다).
// 범위 끝에서 틀리면 그 생태계의 정식 비교로 바꾼다.

use std::cmp::Ordering;

use super::Ecosystem;

#[derive(Debug, PartialEq, Eq)]
enum Token {
    Number(u64),
    /// (순서, 소문자 글자)
    Word(u8, String),
}

/// 정식 배포의 순서(빈 자리·`ga`·`final`).
const RELEASE: u8 = 6;

fn rank(ecosystem: Ecosystem, word: &str) -> u8 {
    match ecosystem {
        Ecosystem::Maven => match word {
            "alpha" | "a" => 1,
            "beta" | "b" => 2,
            "milestone" | "m" => 3,
            "rc" | "cr" => 4,
            "snapshot" => 5,
            "ga" | "final" | "release" => RELEASE,
            "sp" => 7,
            _ => 8,
        },
        Ecosystem::PyPI => match word {
            "dev" => 0,
            "a" | "alpha" => 1,
            "b" | "beta" => 2,
            "c" | "rc" | "pre" | "preview" => 4,
            "post" | "rev" | "r" => 7,
            _ => 3,
        },
        // semver: 사전 배포는 모두 정식 앞이고 그 사이는 사전 순.
        Ecosystem::Npm | Ecosystem::Crates => 1,
    }
}

fn tokens(ecosystem: Ecosystem, version: &str) -> Vec<Token> {
    let version = version.trim().trim_start_matches(['v', 'V']);
    // semver의 빌드 정보는 순서에 들지 않는다.
    let version = match ecosystem {
        Ecosystem::Npm | Ecosystem::Crates => version.split('+').next().unwrap_or(version),
        _ => version,
    };
    let mut out = Vec::new();
    let mut current = String::new();
    let flush = |current: &mut String, out: &mut Vec<Token>| {
        if current.is_empty() {
            return;
        }
        let token = match current.parse::<u64>() {
            Ok(n) => Token::Number(n),
            Err(_) => {
                let word = current.to_ascii_lowercase();
                let rank = rank(ecosystem, &word);
                if rank == RELEASE {
                    current.clear();
                    return;
                }
                Token::Word(rank, word)
            }
        };
        out.push(token);
        current.clear();
    };
    for c in version.chars() {
        if c.is_ascii_alphanumeric() {
            // 숫자와 글자가 바뀌는 곳에서 나눈다(`1rc2` → 1, rc, 2).
            if current
                .chars()
                .last()
                .is_some_and(|last| last.is_ascii_digit() != c.is_ascii_digit())
            {
                flush(&mut current, &mut out);
            }
            current.push(c);
        } else {
            flush(&mut current, &mut out);
        }
    }
    flush(&mut current, &mut out);
    // 끝의 0은 없는 것과 같다(`1.0` == `1.0.0`).
    while matches!(out.last(), Some(Token::Number(0))) {
        out.pop();
    }
    out
}

/// 두 버전을 견준다.
pub fn compare(ecosystem: Ecosystem, a: &str, b: &str) -> Ordering {
    let (a, b) = (tokens(ecosystem, a), tokens(ecosystem, b));
    for i in 0..a.len().max(b.len()) {
        let order = match (a.get(i), b.get(i)) {
            (Some(Token::Number(x)), Some(Token::Number(y))) => x.cmp(y),
            (Some(Token::Word(rx, x)), Some(Token::Word(ry, y))) => {
                rx.cmp(ry).then_with(|| x.cmp(y))
            }
            (Some(Token::Number(_)), Some(Token::Word(..))) => Ordering::Greater,
            (Some(Token::Word(..)), Some(Token::Number(_))) => Ordering::Less,
            // 빈 자리: 숫자 앞이면 0, 글자 앞이면 정식.
            (Some(Token::Number(x)), None) => x.cmp(&0),
            (None, Some(Token::Number(y))) => 0.cmp(y),
            (Some(Token::Word(r, _)), None) => r.cmp(&RELEASE),
            (None, Some(Token::Word(r, _))) => RELEASE.cmp(r),
            (None, None) => Ordering::Equal,
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    Ordering::Equal
}

#[cfg(test)]
mod tests {
    use super::*;
    use Ecosystem::*;

    fn sorted(ecosystem: Ecosystem, versions: &[&str]) {
        for pair in versions.windows(2) {
            assert_eq!(
                compare(ecosystem, pair[0], pair[1]),
                Ordering::Less,
                "{ecosystem:?}: {} < {}",
                pair[0],
                pair[1]
            );
            assert_eq!(compare(ecosystem, pair[1], pair[0]), Ordering::Greater);
        }
    }

    #[test]
    fn 생태계마다_사전_배포_순서를_따른다() {
        sorted(
            Maven,
            &[
                "2.0-beta9",
                "2.0-rc1",
                "2.0",
                "2.0.1",
                "2.3.1",
                "2.12.2",
                "2.13.0",
                "2.14.1",
                "2.15.0-SNAPSHOT",
                "2.15.0",
                "2.15.0-sp1",
                "2.15.1",
                "3.0.0-M1",
            ],
        );
        assert_eq!(compare(Maven, "1.0", "1.0.0"), Ordering::Equal);
        assert_eq!(compare(Maven, "5.3.20.RELEASE", "5.3.20"), Ordering::Equal);
        assert_eq!(compare(Maven, "1.0-ga", "1.0-final"), Ordering::Equal);
        sorted(
            PyPI,
            &[
                "1.0.dev1",
                "1.0a1",
                "1.0b2",
                "1.0rc1",
                "1.0",
                "1.0.post1",
                "1.1",
            ],
        );
        sorted(
            Npm,
            &[
                "1.0.0-alpha",
                "1.0.0-alpha.1",
                "1.0.0-beta",
                "1.0.0",
                "1.0.1",
                "1.10.0",
            ],
        );
        assert_eq!(compare(Npm, "1.0.0+build.5", "1.0.0"), Ordering::Equal);
        sorted(Crates, &["0.9.9", "1.0.0-rc.1", "1.0.0", "1.2.3"]);
        assert_eq!(compare(Crates, "v1.2", "1.2.0"), Ordering::Equal);
    }
}
