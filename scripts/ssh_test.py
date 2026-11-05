"""Start/stop an isolated SSH fixture. Keys and config stay in ignored dist/ssh-test."""
import argparse
from pathlib import Path
import subprocess
import time


def run(*args, **kwargs):
    return subprocess.check_output(args, text=True, **kwargs).strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["up", "down"])
    parser.add_argument("--port", type=int, default=2222)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    state = root / "dist" / "ssh-test"
    name = "patchlook-ssh-test"
    image = "patchlook-ssh-test:local"
    ids = run("docker", "ps", "-aq", "--filter", f"name=^/{name}$")
    if ids:
        label = run("docker", "inspect", "--format", '{{index .Config.Labels "dev.patchlook.fixture"}}', name)
        if label != "ssh":
            raise SystemExit("The container name is already used by another workload")
    if args.action == "down":
        if ids:
            run("docker", "stop", name)
        return
    state.mkdir(parents=True, exist_ok=True)
    key = state / "id_ed25519"
    if not key.exists():
        run("ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-C", "patchlook-ssh-test", "-f", str(key))
    encrypted_key = state / "id_ed25519_encrypted"
    if not encrypted_key.exists():
        run("ssh-keygen", "-q", "-t", "ed25519", "-N", "patchlook-test-key", "-C", "patchlook-encrypted-test", "-f", str(encrypted_key))
    if not ids:
        subprocess.run(["docker", "build", "-t", image, str(root / "scripts" / "ssh-test")], check=True)
        run("docker", "run", "-d", "--name", name, "--label", "dev.patchlook.fixture=ssh",
            "-p", f"127.0.0.1:{args.port}:22", "--mount", f"type=bind,src={key}.pub,dst=/run/test-key.pub,readonly", image)
    else:
        run("docker", "cp", str(root / "scripts" / "ssh-test" / "start.sh"), f"{name}:/opt/patchlook/start.sh")
        run("docker", "cp", str(root / "scripts" / "ssh-test" / "seed.py"), f"{name}:/opt/patchlook/seed.py")
        run("docker", "restart", name)
    port = run("docker", "port", name, "22/tcp").rsplit(":", 1)[1]
    for _ in range(60):
        ready = subprocess.run(["docker", "exec", name, "test", "-f", "/home/review/repos/demo/preview.png"], capture_output=True)
        if ready.returncode == 0:
            break
        time.sleep(1)
    else:
        raise SystemExit("SSH fixture did not become ready; inspect docker logs patchlook-ssh-test")
    public = run("docker", "exec", name, "cat", "/etc/ssh/ssh_host_ed25519_key.pub")
    known = state / "known_hosts"
    known.write_text(f"[127.0.0.1]:{port} {public}\n")
    config = state / "config"
    config.write_text(f'''Host patchlook-test
    HostName 127.0.0.1
    Port {port}
    User review
    IdentityFile "{key}"
    UserKnownHostsFile "{known}"
    IdentitiesOnly yes
    StrictHostKeyChecking yes
Host 127.0.0.1
    UserKnownHostsFile "{known}"
    StrictHostKeyChecking yes
''')
    config.chmod(0o600)
    print(f"SSH config: {config}\nHost: patchlook-test\nRepository: /home/review/repos/demo")
    for _ in range(10):
        check = subprocess.run(["ssh", "-F", str(config), "-o", "BatchMode=yes", "patchlook-test",
                                "git -C /home/review/repos/demo status --short"], capture_output=True, text=True)
        if check.returncode == 0:
            print(check.stdout.strip())
            break
        time.sleep(1)
    else:
        raise SystemExit(check.stderr)
    public_keys = key.with_suffix(".pub").read_text() + encrypted_key.with_suffix(".pub").read_text()
    subprocess.run(["docker", "exec", "-i", "-u", "review", name, "sh", "-c", "cat > ~/.ssh/authorized_keys"], input=public_keys, text=True, check=True)


if __name__ == "__main__":
    main()
