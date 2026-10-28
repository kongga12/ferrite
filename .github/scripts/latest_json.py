"""릴리스 매니페스트(latest.json)를 만든다. 앱의 자동 업데이트(crates/ferrite-update)가 읽는다.

형식은 cargo-packager 업데이터와 같다: 플랫폼(OS-아키텍처)마다 설치 파일 주소, `.sig` 내용, 형식.
NSIS·AppImage는 앱이 받아 바꾸므로 서명이 꼭 있어야 하고, dmg는 내려받기 링크로만 쓴다.

사용: python3 latest_json.py <설치 파일 폴더> <태그(v0.2.0)> <저장소(owner/name)>  > latest.json
"""
import datetime
import json
import os
import sys

# (플랫폼, 파일 이름 끝, 형식, 서명 필수)
TARGETS = [
    ("windows-x86_64", "_x64-setup.exe", "nsis", True),
    ("linux-x86_64", "_x86_64.AppImage", "appimage", True),
    ("macos-aarch64", "_aarch64.dmg", "dmg", False),
]


def main(dist, tag, repo):
    version = tag[1:] if tag.startswith("v") else tag
    base = "https://github.com/%s/releases/download/%s/" % (repo, tag)
    files = os.listdir(dist)
    platforms = {}
    for platform, suffix, fmt, required in TARGETS:
        names = [n for n in files if n.endswith(suffix) and "_%s_" % version in n]
        if len(names) != 1:
            sys.exit("%s: %s 판 설치 파일(*%s)을 하나 찾지 못했습니다: %s" % (platform, version, suffix, names))
        name = names[0]
        sig = os.path.join(dist, name + ".sig")
        if not os.path.exists(sig):
            if required:
                sys.exit("%s: 서명 파일이 없습니다: %s.sig" % (platform, name))
            continue
        with open(sig, encoding="ascii") as f:
            signature = f.read().strip()
        platforms[platform] = {"url": base + name, "signature": signature, "format": fmt}
    manifest = {
        "version": version,
        "notes": "",
        "pub_date": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "platforms": platforms,
    }
    json.dump(manifest, sys.stdout, indent=2)
    sys.stdout.write("\n")


if __name__ == "__main__":
    if len(sys.argv) != 4:
        sys.exit(__doc__)
    main(*sys.argv[1:4])
