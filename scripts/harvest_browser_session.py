"""Elevated helper: read flex.team cookies from all Chrome/Edge profiles."""
from __future__ import annotations

import json
import os
import shutil
import sys
import tempfile
import time
from pathlib import Path


def session_path() -> Path:
    base = Path(os.environ.get("LOCALAPPDATA", ".")) / "FlexWorkWidget"
    base.mkdir(parents=True, exist_ok=True)
    return base / "session.json"


def ensure_rookie():
    try:
        import rookiepy  # noqa: F401
        return
    except ImportError:
        import subprocess

        subprocess.check_call([sys.executable, "-m", "pip", "install", "--user", "rookiepy"])


def profile_dirs(user_data: Path) -> list[Path]:
    if not user_data.is_dir():
        return []
    out: list[Path] = []
    for p in user_data.iterdir():
        if not p.is_dir():
            continue
        if p.name == "Default" or p.name.startswith("Profile "):
            out.append(p)
    return out


def last_active_map(user_data: Path) -> dict[str, float]:
    local_state = user_data / "Local State"
    if not local_state.exists():
        return {}
    try:
        data = json.loads(local_state.read_text(encoding="utf-8"))
        cache = data.get("profile", {}).get("info_cache", {}) or {}
        return {
            name: float(info.get("active_time") or 0)
            for name, info in cache.items()
            if isinstance(info, dict)
        }
    except Exception:
        return {}


def cookie_db_for(profile: Path) -> Path | None:
    for rel in ("Network/Cookies", "Cookies"):
        p = profile / rel
        if p.exists():
            return p
    return None


def browsers() -> list[tuple[str, Path]]:
    local = Path(os.environ.get("LOCALAPPDATA", ""))
    # Edge first: app-bound encryption is usually less painful than Chrome.
    return [
        ("edge", local / "Microsoft" / "Edge" / "User Data"),
        ("chrome", local / "Google" / "Chrome" / "User Data"),
        ("brave", local / "BraveSoftware" / "Brave-Browser" / "User Data"),
        ("chromium", local / "Chromium" / "User Data"),
    ]


def read_profile_cookies(key_path: Path, db_path: Path, domains: list[str]) -> list[dict]:
    from rookiepy import chromium_based

    # Chrome locks Cookies while running — copy first.
    with tempfile.TemporaryDirectory(prefix="flex-cookies-") as tmp:
        tmp_db = Path(tmp) / "Cookies"
        shutil.copy2(db_path, tmp_db)
        wal = Path(str(db_path) + "-wal")
        shm = Path(str(db_path) + "-shm")
        if wal.exists():
            shutil.copy2(wal, Path(tmp) / "Cookies-wal")
        if shm.exists():
            shutil.copy2(shm, Path(tmp) / "Cookies-shm")
        return list(chromium_based(str(key_path), str(tmp_db), domains) or [])


def collect_candidates() -> tuple[list[dict], list[str]]:
    """Return (candidates with AID/V2_WS_AID, errors). Prefer most recently active profiles."""
    ensure_rookie()
    domains = ["flex.team"]
    errors: list[str] = []
    candidates: list[dict] = []

    for browser, user_data in browsers():
        if not user_data.is_dir():
            continue
        key_path = user_data / "Local State"
        if not key_path.exists():
            errors.append(f"{browser}: Local State 없음")
            continue
        active = last_active_map(user_data)
        profiles = profile_dirs(user_data)
        profiles.sort(key=lambda p: active.get(p.name, 0.0), reverse=True)

        for profile in profiles:
            db = cookie_db_for(profile)
            if db is None:
                continue
            label = f"{browser}/{profile.name}"
            try:
                cookies = read_profile_cookies(key_path, db, domains)
            except Exception as e:
                msg = str(e).split("Location:", 1)[0].strip()
                errors.append(f"{label}: {msg}")
                continue

            found: dict[str, str] = {}
            for c in cookies:
                n = c.get("name") or ""
                v = c.get("value") or ""
                if n in ("AID", "V2_WS_AID", "V2_WS_RID") and v:
                    found[n] = v
            if found.get("AID") or found.get("V2_WS_AID"):
                candidates.append(
                    {
                        "aid": found.get("AID", ""),
                        "wsAid": found.get("V2_WS_AID", ""),
                        "wsRid": found.get("V2_WS_RID", ""),
                        "source": label,
                        "active": active.get(profile.name, 0.0),
                    }
                )

    # Edge/Chrome defaults as last resort (single-profile APIs)
    from rookiepy import chrome, edge

    for name, fn in (("edge", edge), ("chrome", chrome)):
        try:
            cookies = fn(domains)
        except Exception as e:
            msg = str(e).split("Location:", 1)[0].strip()
            errors.append(f"{name}(): {msg}")
            continue
        found = {}
        for c in cookies:
            n = c.get("name") or ""
            v = c.get("value") or ""
            if n in ("AID", "V2_WS_AID", "V2_WS_RID") and v:
                found[n] = v
        if found.get("AID") or found.get("V2_WS_AID"):
            candidates.append(
                {
                    "aid": found.get("AID", ""),
                    "wsAid": found.get("V2_WS_AID", ""),
                    "wsRid": found.get("V2_WS_RID", ""),
                    "source": f"{name}()",
                    "active": -1.0,
                }
            )

    # Prefer highest active_time within browser preference order already collected
    candidates.sort(key=lambda c: c.get("active", 0.0), reverse=True)
    return candidates, errors


def collect() -> dict:
    candidates, errors = collect_candidates()
    if not candidates:
        hint = (
            "flex.team 로그인 쿠키를 찾지 못했습니다.\n"
            "- Chrome 프로필이 여러 개면, flex에 로그인한 프로필을 확인하세요.\n"
            "- Chrome v130+는 관리자(UAC) 허용이 필요합니다.\n"
            "- 그래도 실패하면 Edge에서 flex.team에 로그인한 뒤 다시 시도하세요."
        )
        if errors:
            # Keep message short for UI
            uniq = []
            for e in errors:
                if e not in uniq:
                    uniq.append(e)
            hint += "\n\n상세: " + " | ".join(uniq[:6])
        raise RuntimeError(hint)

    best = candidates[0]
    return {
        "aid": best.get("aid", ""),
        "wsAid": best.get("wsAid", ""),
        "wsRid": best.get("wsRid", ""),
        "userIdHash": None,
        "updatedAtMs": int(time.time() * 1000),
        "source": best.get("source"),
    }


def main() -> int:
    out = session_path()
    try:
        payload = collect()
        out.write_text(json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8")
        err_path = out.with_name("session-error.txt")
        if err_path.exists():
            err_path.unlink()
        print(f"OK wrote {out} source={payload.get('source')}")
        return 0
    except Exception as e:
        err_path = out.with_name("session-error.txt")
        err_path.write_text(str(e), encoding="utf-8")
        print(f"ERR {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
