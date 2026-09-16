#!/usr/bin/env python3
"""Exercise the mutation supervisor through adb/su using only a unique scratch directory.

Run with uv run scripts/test-root-transport.py --serial SERIAL --binary PATH.
Does not install the APK, invoke an activator or access the production config.
"""

import argparse
import json
import shlex
import subprocess
import time
import uuid
from pathlib import Path


class DeviceFixture:
    def __init__(self, serial: str, binary: Path):
        self.adb = ["adb", "-s", serial]
        self.root = f"/data/local/tmp/vpnhide-state-test-{uuid.uuid4()}"
        staged = f"{self.root}.bin"
        subprocess.run([*self.adb, "push", str(binary), staged], check=True)
        self.shell(
            f"mkdir {self.root} && chmod 700 {self.root} && "
            f"cp {staged} {self.root}/helper && chmod 700 {self.root}/helper && "
            f"rm {staged} && printf '{{}}' >{self.root}/config"
        )
        self.session = str(uuid.uuid4())
        initial = self.call("inspect")
        self.boot = initial["boot"]
        assert initial["state"]["revision"] == 0, initial
        # A lane never opened in this boot is adopted in one round trip (the app's startup path).
        adopted = self.call("adopt", self.session)
        assert adopted["status"] == "ok" and adopted["state"]["session"] == self.session, adopted
        # A repeated adoption of the tracked session acknowledges it without resetting it.
        assert self.call("adopt", self.session)["state"]["session"] == self.session

    def command(self, command: str) -> list[str]:
        return [*self.adb, "shell", "-T", f"su -c {shlex.quote(command)}"]

    def shell(self, command: str, check: bool = True) -> subprocess.CompletedProcess:
        return subprocess.run(
            self.command(command), capture_output=True, text=True, timeout=10, check=check
        )

    def helper(self, args: tuple[str, ...]) -> str:
        return shlex.join(
            [f"{self.root}/helper", f"{self.root}/lane", f"{self.root}/config", *args]
        )

    def call(self, *args: str, script: str = "") -> dict:
        if args[0] == "run":
            script = frame_script(script)
        result = subprocess.run(
            self.command(self.helper(args)),
            input=script,
            capture_output=True,
            text=True,
            timeout=10,
            check=True,
        )
        return json.loads(result.stdout)

    def phase(self, sequence: int, body: str) -> dict:
        return self.call(
            "run", self.boot, self.session, str(sequence), script=f"cd {self.root}\n{body}\n"
        )

    def start(self, sequence: int, body: str) -> subprocess.Popen:
        command = f"echo $$ >{self.root}/supervisor-pid; exec " + self.helper(
            ("run", self.boot, self.session, str(sequence))
        )
        process = subprocess.Popen(
            self.command(command),
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        assert process.stdin is not None
        process.stdin.write(frame_script(f"cd {self.root}\n{body}\n"))
        process.stdin.close()
        process.stdin = None
        return process

    def await_file(self, name: str):
        deadline = time.monotonic() + 5
        while self.shell(f"test -f {self.root}/{name}", check=False).returncode:
            if time.monotonic() > deadline:
                raise TimeoutError(
                    f"missing device handshake: {name}; fixture retained at {self.root}"
                )
            time.sleep(0.02)


def frame_script(script: str) -> str:
    return f"vpnhide-script 1 {len(script.encode('utf-8'))}\n{script}"


def test_receipts(fixture: DeviceFixture):
    completed = fixture.phase(1, "printf changed >config")
    assert completed["state"]["status"] == "finished", completed
    assert completed["state"]["exit_code"] == 0, completed
    assert completed["config"] == "changed", completed
    recovered = fixture.call("recover", fixture.boot, fixture.session, "1")
    assert recovered["state"] == completed["state"], recovered
    assert fixture.phase(1, "printf stale >config")["status"] == "rejected"
    fenced = fixture.call("recover", fixture.boot, fixture.session, "2")
    assert fenced["state"]["status"] == "not_started", fenced
    assert fixture.phase(2, "printf stale >config")["status"] == "rejected"
    partial = fixture.phase(
        3,
        "printf partial >config; head -c 1048576 /dev/zero; echo; echo synthetic-private; "
        "echo 'vpnhide-warning native_target_cap total=10 cap=8 dropped=2' >&2; exit 7",
    )
    assert partial["state"]["exit_code"] == 7 and partial["config"] == "partial", partial
    assert "synthetic-private" not in json.dumps(partial)
    assert "synthetic-private" not in fixture.shell(f"cat {fixture.root}/lane/state.json").stdout
    expected = {"total": 10, "cap": 8, "dropped": 2}
    assert partial["state"]["native_capacity"] == expected, partial
    assert (
        fixture.call("recover", fixture.boot, fixture.session, "3")["state"]["native_capacity"]
        == expected
    )
    print(
        "PASS durable receipt, stale launch rejection, recovery fence, "
        "partial write, redaction, bounded capacity evidence"
    )


def test_descendants(fixture: DeviceFixture):
    process = fixture.start(
        4,
        "(touch entered; i=0; "
        "while [ ! -f release ] && [ $i -lt 250 ]; do sleep 0.02; i=$((i+1)); done; "
        "[ -f release ] || exit 124; "
        "printf child >config) &\nexit 0",
    )
    try:
        fixture.await_file("entered")
        assert fixture.call("inspect")["status"] == "busy"
        assert fixture.call("recover", fixture.boot, fixture.session, "4")["status"] == "busy"
    finally:
        fixture.shell(f"touch {fixture.root}/release")
        output, error = process.communicate(timeout=10)
    assert process.returncode == 0, error
    completed = json.loads(output)
    assert completed["state"]["status"] == "finished" and completed["config"] == "child", completed
    print("PASS detached descendant retains lane until its write completes")


def test_supervisor_loss(fixture: DeviceFixture):
    process = fixture.start(
        5,
        "touch entered-kill; i=0; "
        "while [ ! -f release-kill ] && [ $i -lt 250 ]; do sleep 0.02; i=$((i+1)); done; "
        "[ -f release-kill ] || exit 124; "
        "printf late >config; touch done-kill",
    )
    try:
        fixture.await_file("entered-kill")
        pid = int(fixture.shell(f"cat {fixture.root}/supervisor-pid").stdout.strip())
        fixture.shell(f"kill -KILL {pid}")
        process.communicate(timeout=10)
        recovered = fixture.call("recover", fixture.boot, fixture.session, "5")
        assert recovered["state"]["status"] == "running", recovered
        assert recovered["config_status"] == "unavailable", recovered
        assert fixture.phase(6, "printf unsafe >config")["status"] == "rejected"
    finally:
        fixture.shell(f"touch {fixture.root}/release-kill")
        fixture.await_file("done-kill")
        if process.poll() is None:
            process.communicate(timeout=10)
    assert fixture.call("inspect")["state"]["status"] == "running"
    print("PASS supervisor death stays unresolved even after child exit; no PID-based recovery")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial", required=True)
    parser.add_argument("--binary", required=True, type=Path)
    args = parser.parse_args()
    fixture = DeviceFixture(args.serial, args.binary)
    print(f"Fixture: {fixture.root}", flush=True)
    test_receipts(fixture)
    test_descendants(fixture)
    test_supervisor_loss(fixture)
    # The test knows its only descendant reached done-kill. Production recovery has no such proof.
    fixture.shell(f"rm -r {fixture.root}")
    print("PASS device transport checks; scratch files removed")


if __name__ == "__main__":
    main()
