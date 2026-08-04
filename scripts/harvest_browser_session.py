"""Elevated helper: read flex.team cookies from Chrome/Edge into session.json."""
from __future__ import annotations

import json
import os
import sys
import time
from pathlib import Path


def session_path() -> Path:
    base = Path(os.environ.get("LOCALAPPDATA", ".")) / "FlexWorkWidget"
    base.mkdir(parents=True, exist_ok=True)
    return base / "session.json"


def collect() -> dict:
    from rookiepy import chrome, edge

    domains = ["flex.team"]
    errors: list[str] = []
    found: dict[str, str] = {}

    for name, fn in (("chrome", chrome), ("edge", edge)):
        try:
            cookies = fn(domains)
        except Exception as e:
            errors.append(f"{name}: {e}")
            continue
        for c in cookies:
            n = c.get("name") or ""
            v = c.get("value") or ""
            if n in ("AID", "V2_WS_AID", "V2_WS_RID") and v:
                # Prefer non-empty; later browser can override
                found[n] = v
        if found.get("AID") or found.get("V2_WS_AID"):
            found["_source"] = name
            break

    if not (found.get("AID") or found.get("V2_WS_AID")):
        raise RuntimeError(
            "flex.team 로그인 쿠키를 찾지 못했습니다. "
            "Chrome/Edge에서 flex.team에 로그인한 뒤 다시 시도하세요. "
            + (" | ".join(errors) if errors else "")
        )

    return {
        "aid": found.get("AID", ""),
        "wsAid": found.get("V2_WS_AID", ""),
        "wsRid": found.get("V2_WS_RID", ""),
        "userIdHash": None,
        "updatedAtMs": int(time.time() * 1000),
        "source": found.get("_source"),
    }


def main() -> int:
    out = session_path()
    try:
        payload = collect()
        out.write_text(json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8")
        print(f"OK wrote {out}")
        return 0
    except Exception as e:
        err_path = out.with_name("session-error.txt")
        err_path.write_text(str(e), encoding="utf-8")
        print(f"ERR {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
