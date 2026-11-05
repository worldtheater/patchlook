"""Disposable Linux repositories for SSH browsing, with no host repository mounts."""
from pathlib import Path
import base64
import subprocess

root = Path.home() / "repos"
root.mkdir(exist_ok=True)


def git(repo, *args):
    return subprocess.check_output(["git", "-C", str(repo), *args])


def init(name):
    repo = root / name
    repo.mkdir()
    git(repo, "init", "-b", "main")
    git(repo, "config", "user.name", "Patchlook SSH Test")
    git(repo, "config", "user.email", "ssh@example.test")
    git(repo, "config", "commit.gpgsign", "false")
    return repo


def seed_demo():
    repo = init("demo")
    for name, data in {"partial.txt": b"one\nkeep\ntwo\nkeep\nthree\n", "deleted.txt": b"deleted\n", "old name.txt": b"moved\n", "binary.bin": b"\0before"}.items():
        (repo / name).write_bytes(data)
    git(repo, "add", "--all")
    git(repo, "commit", "-m", "Initial SSH fixture")
    for i in range(105):
        git(repo, "commit", "--allow-empty", "-m", f"History sample {i:03d}", "-m", "Searchable body: remote-review")
    git(repo, "branch", "feature/demo")
    (repo / "partial.txt").write_text("ONE\nkeep\ntwo\nkeep\nthree\n")
    git(repo, "add", "partial.txt")
    git(repo, "mv", "old name.txt", "moved name.txt")
    (repo / "partial.txt").write_text("ONE\nkeep\nTWO\nkeep\nthree\n")
    (repo / "deleted.txt").unlink()
    (repo / "binary.bin").write_bytes(b"\0after")
    (repo / "中文 ' [literal].txt").write_text("Untracked UTF-8\nno final newline")
    (repo / "$(touch SHOULD_NOT_EXIST).txt").write_text("literal shell characters\n")
    (repo / "empty.txt").touch()
    (repo / "preview.png").write_bytes(base64.b64decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII="))
    unborn = init("unborn")
    (unborn / "new.txt").write_text("No commits yet\n")


def seed_submodules():
    # A separate parent keeps existing SSH diff/count fixtures unchanged.
    module_source = init("module-source")
    (module_source / "child.txt").write_text("Child repository\n")
    git(module_source, "add", "child.txt")
    git(module_source, "commit", "-m", "Child history")
    modules = init("modules")
    for path in ("deps/子模块 [one]", "missing"):
        git(modules, "-c", "protocol.file.allow=always", "submodule", "add", str(module_source), path)
    git(modules, "commit", "-am", "Submodule parent")
    git(modules, "submodule", "deinit", "-f", "--", "missing")
    (modules / "deps/子模块 [one]/new.txt").write_text("Untracked child file\n")


if not (root / "demo/.git").is_dir():
    seed_demo()
if not (root / "modules/.git").is_dir():
    seed_submodules()
