"""Elevated helper: read flex.team cookies from Chrome/Edge profiles.

Selection favours the profile the user is *currently* using — the one Chrome/Edge
records as ``last_used`` / ``last_active_profiles`` in ``Local State`` — instead of
just the profile with the newest ``active_time``. This matches "use the Chrome
window I just had open" and fixes wrong-profile picks when flex is logged into
several profiles.

Usage:
    harvest_browser_session.py [--prefer <browser>/<profile>]

Outputs (under %LOCALAPPDATA%\\FlexWorkWidget):
    session.json     cookies for the chosen profile (AID / V2_WS_AID / V2_WS_RID)
    candidates.json  metadata list of every profile that has a flex session
                     (browser, profile, account label — NO cookie values)
    session-error.txt written on failure, removed on success
"""
from __future__ import annotations

import argparse
import json
import os
import shutil
import sys
import tempfile
import time
from pathlib import Path


def base_dir() -> Path:
    base = Path(os.environ.get("LOCALAPPDATA", ".")) / "FlexWorkWidget"
    base.mkdir(parents=True, exist_ok=True)
    return base


def session_path() -> Path:
    return base_dir() / "session.json"


def candidates_path() -> Path:
    return base_dir() / "candidates.json"


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


def read_local_state(user_data: Path) -> dict:
    local_state = user_data / "Local State"
    if not local_state.exists():
        return {}
    try:
        return json.loads(local_state.read_text(encoding="utf-8"))
    except Exception:
        return {}


def profile_info(local_state: dict) -> dict:
    """profile name -> {active_time, account, display}."""
    cache = (local_state.get("profile", {}) or {}).get("info_cache", {}) or {}
    out: dict[str, dict] = {}
    for name, info in cache.items():
        if not isinstance(info, dict):
            continue
        out[name] = {
            "active_time": float(info.get("active_time") or 0),
            # user_name is the signed-in account email; name is the label.
            "account": (info.get("user_name") or "").strip(),
            "display": (info.get("name") or "").strip(),
        }
    return out


def last_used_info(local_state: dict) -> tuple[str, set[str]]:
    prof = local_state.get("profile", {}) or {}
    last_used = str(prof.get("last_used") or "")
    active = prof.get("last_active_profiles") or []
    if isinstance(active, list):
        active_set = {str(x) for x in active}
    else:
        active_set = set()
    return last_used, active_set


def cookie_db_for(profile: Path) -> Path | None:
    for rel in ("Network/Cookies", "Cookies"):
        p = profile / rel
        if p.exists():
            return p
    return None


def browsers() -> list[tuple[str, Path]]:
    local = Path(os.environ.get("LOCALAPPDATA", ""))
    return [
        ("chrome", local / "Google" / "Chrome" / "User Data"),
        ("edge", local / "Microsoft" / "Edge" / "User Data"),
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
    """Return (candidates, errors).

    Each candidate carries its flex cookies plus ranking signals:
    ``isLastUsed`` (profile Chrome/Edge would open a new page into),
    ``inLastActive`` and ``active`` (raw active_time).
    """
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

        local_state = read_local_state(user_data)
        info = profile_info(local_state)
        last_used, last_active = last_used_info(local_state)

        profiles = profile_dirs(user_data)
        for profile in profiles:
            db = cookie_db_for(profile)
            if db is None:
                continue
            pname = profile.name
            label = f"{browser}/{pname}"
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
            if not (found.get("AID") or found.get("V2_WS_AID")):
                continue

            meta = info.get(pname, {})
            account = meta.get("account") or meta.get("display") or pname
            candidates.append(
                {
                    "id": label,
                    "browser": browser,
                    "profile": pname,
                    "account": account,
                    "aid": found.get("AID", ""),
                    "wsAid": found.get("V2_WS_AID", ""),
                    "wsRid": found.get("V2_WS_RID", ""),
                    "active": meta.get("active_time", 0.0),
                    "isLastUsed": pname == last_used,
                    "inLastActive": pname in last_active,
                }
            )

    return candidates, errors


def rank_candidates(candidates: list[dict], prefer: str | None) -> list[dict]:
    def key(c: dict):
        return (
            1 if prefer and c["id"] == prefer else 0,  # explicit user choice wins
            1 if c.get("isLastUsed") else 0,            # profile currently in use
            1 if c.get("inLastActive") else 0,          # was active last session
            c.get("active", 0.0),                       # most recently opened
        )

    return sorted(candidates, key=key, reverse=True)


def write_candidates(ranked: list[dict]) -> None:
    """Persist metadata only — never cookie values."""
    meta = [
        {
            "id": c["id"],
            "browser": c["browser"],
            "profile": c["profile"],
            "account": c["account"],
            "isLastUsed": bool(c.get("isLastUsed")),
        }
        for c in ranked
    ]
    candidates_path().write_text(
        json.dumps(meta, ensure_ascii=False, indent=2), encoding="utf-8"
    )


def collect(prefer: str | None) -> dict:
    candidates, errors = collect_candidates()
    ranked = rank_candidates(candidates, prefer)
    write_candidates(ranked)

    if not ranked:
        hint = (
            "flex.team 로그인 쿠키를 찾지 못했습니다.\n"
            "- 크롬/엣지에서 flex.team에 로그인한 프로필이 있는지 확인하세요.\n"
            "- 크롬 v130+는 관리자(UAC) 허용이 필요합니다.\n"
            "- 여러 프로필이면, flex에 로그인한 프로필을 사용 중인 상태로 두고 다시 시도하세요."
        )
        if errors:
            uniq: list[str] = []
            for e in errors:
                if e not in uniq:
                    uniq.append(e)
            hint += "\n\n상세: " + " | ".join(uniq[:6])
        raise RuntimeError(hint)

    best = ranked[0]
    return {
        "aid": best.get("aid", ""),
        "wsAid": best.get("wsAid", ""),
        "wsRid": best.get("wsRid", ""),
        "userIdHash": None,
        "updatedAtMs": int(time.time() * 1000),
        "source": best.get("id"),
        "account": best.get("account"),
    }


def parse_args(argv: list[str]) -> argparse.Namespace:
    p = argparse.ArgumentParser(description="Harvest flex.team session cookies")
    p.add_argument(
        "--prefer",
        default=None,
        help="Preferred profile id, e.g. chrome/Default",
    )
    return p.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    prefer = args.prefer or os.environ.get("FLEX_PREFER_PROFILE") or None
    out = session_path()
    try:
        payload = collect(prefer)
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
