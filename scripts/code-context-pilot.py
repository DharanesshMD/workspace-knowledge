#!/usr/bin/env python3
"""Five-identity loopback pilot for the advisory code-context endpoint."""
import concurrent.futures
import json
import math
import os
import statistics
import subprocess
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path


def percentile(values, p):
    ordered = sorted(values)
    return round(ordered[min(len(ordered) - 1, math.ceil(p * len(ordered)) - 1)], 2)


def main():
    fixture_path = os.environ.get("WK_CODE_CONTEXT_PILOT_FIXTURE")
    if not fixture_path:
        print(json.dumps({"status": "unverified", "reason": "WK_CODE_CONTEXT_PILOT_FIXTURE is not configured"}))
        return
    fixture_file = Path(fixture_path)
    if not fixture_file.is_file():
        raise SystemExit("Pilot fixture file does not exist")
    if os.name != "nt" and fixture_file.stat().st_mode & 0o077:
        raise SystemExit("Pilot fixture must be private (mode 0600 or stricter)")
    db_url = os.environ.get("WK_TEST_DATABASE_URL", "")
    if urllib.parse.urlparse(db_url).path != "/wk_test":
        raise SystemExit("Only the disposable /wk_test database is accepted")
    fixture = json.loads(fixture_file.read_text(encoding="utf-8"))
    base = fixture["base_url"].rstrip("/")
    parsed = urllib.parse.urlparse(base)
    if parsed.scheme != "http" or parsed.hostname not in ("127.0.0.1", "localhost", "::1"):
        raise SystemExit("Pilot endpoint must be an HTTP loopback synthetic server")
    users = fixture.get("users", [])
    if len(users) != 5 or len({u.get("token") for u in users}) != 5:
        raise SystemExit("Pilot requires five distinct synthetic developer tokens")
    rounds = fixture.get("rounds", 20)
    if not isinstance(rounds, int) or not 1 <= rounds <= 1000:
        raise SystemExit("rounds must be between 1 and 1000")
    endpoint = base + "/v1/products/" + urllib.parse.quote(fixture["product"], safe="") + "/change-context"
    sizes_before = relation_sizes(db_url)

    def request(user):
        body = json.dumps(user["request"], separators=(",", ":")).encode()
        req = urllib.request.Request(endpoint, data=body, method="POST", headers={
            "Authorization": "Bearer " + user["token"],
            "Content-Type": "application/json",
        })
        start = time.perf_counter()
        try:
            with urllib.request.urlopen(req, timeout=5) as response:
                data = response.read(16385)
                if response.status != 200 or len(data) > 16384:
                    raise RuntimeError("unexpected status or oversized context response")
                decoded = json.loads(data)
                if "PRIVATE_CANARY" in data.decode("utf-8", "replace"):
                    raise RuntimeError("private canary reached shared response")
                return (time.perf_counter() - start) * 1000, len(data), decoded
        except (urllib.error.URLError, TimeoutError) as exc:
            raise RuntimeError("synthetic request failed or timed out") from exc

    cold = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=5) as pool:
        cold = list(pool.map(request, users))
    warm = []
    for _ in range(rounds):
        with concurrent.futures.ThreadPoolExecutor(max_workers=5) as pool:
            warm.extend(pool.map(request, users))
    sizes_after = relation_sizes(db_url)
    latencies = [x[0] for x in warm]
    print(json.dumps({
        "status": "measured",
        "identities": 5,
        "warm_rounds_per_identity": rounds,
        "checks": {"cold_requests": len(cold), "warm_requests": len(warm), "responses_within_16KiB": True, "private_canary_absent": True},
        "latency_ms": {"cold_p50": round(statistics.median(x[0] for x in cold), 2), "warm_p50": round(statistics.median(latencies), 2), "warm_p95": percentile(latencies, .95)},
        "response_bytes": {"max": max(x[1] for x in cold + warm)},
        "postgres_relation_bytes_before": sizes_before,
        "postgres_relation_bytes_after": sizes_after,
        "postgres_relation_byte_delta": {key: sizes_after.get(key, 0) - sizes_before.get(key, 0) for key in sorted(set(sizes_before) | set(sizes_after))},
        "capacity_claim": "none; bounded loopback pilot only",
    }, sort_keys=True))


def relation_sizes(db_url):
    query = "SELECT coalesce(json_object_agg(relname, pg_total_relation_size(quote_ident(schemaname)||'.'||quote_ident(relname))), '{}'::json)::text FROM pg_stat_user_tables WHERE schemaname='public' AND relname IN ('code_preflights','code_repositories','code_mappings','code_contributions','code_rationales')"
    result = subprocess.run(["psql", db_url, "-X", "-At", "-v", "ON_ERROR_STOP=1", "-c", query], capture_output=True, text=True, timeout=5)
    if result.returncode:
        raise SystemExit("Cannot read synthetic PostgreSQL relation sizes")
    return {key: int(value) for key, value in json.loads(result.stdout).items()}


if __name__ == "__main__":
    main()
