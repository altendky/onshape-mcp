#!/usr/bin/env sh
# Exercise binary discovery and linking checks with real ELF executables.
set -eu

TARGET=${1:?Usage: test-verify-static-linking.sh MUSL_TARGET}
python - "$(dirname "$0")/verify-static-linking.sh" "$TARGET" <<'PY'
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

VERIFIER = Path(sys.argv.pop(1)).resolve()
TARGET = sys.argv.pop(1)


class StaticLinkingTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.directory = tempfile.TemporaryDirectory(prefix="static-linking-")
        cls.addClassCleanup(cls.directory.cleanup)
        cls.root = Path(cls.directory.name)
        source = cls.root / "fixture.c"
        source.write_text('#include <stdio.h>\nint main(void) { puts("fixture"); }\n')
        cls.static = cls.root / "directory with spaces" / "static binary"
        cls.static.parent.mkdir()
        cls.dynamic = cls.root / "dynamic"
        for flags, output in [(["-static"], cls.static), ([], cls.dynamic)]:
            subprocess.run(["cc", *flags, str(source), "-o", str(output)], check=True)

    def metadata(self, *binaries):
        return {
            "rust-build-meta": {
                "target-directory": str(self.root),
                "non-test-binaries": {},
            },
            "rust-binaries": {
                str(index): {"binary-path": str(path), "build-platform": "target"}
                for index, path in enumerate(binaries)
            },
        }

    def verify(self, metadata):
        path = self.root / f"{self.id()}.json"
        path.write_text(json.dumps(metadata), encoding="utf-8")
        return subprocess.run(
            [str(VERIFIER), str(path)], capture_output=True, text=True
        )

    def test_static_binary_with_spaces(self):
        result = self.verify(self.metadata(self.static))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Checked 1 target executables", result.stdout)

    def test_dynamic_binary_fails(self):
        result = self.verify(self.metadata(self.dynamic))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("dynamically linked", result.stderr)

    def test_missing_binary_fails(self):
        result = self.verify(self.metadata(self.root / "missing"))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("missing or non-executable", result.stderr)

    def test_empty_metadata_fails(self):
        result = self.verify(self.metadata())
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("no target executables", result.stderr)

    def test_malformed_metadata_fails(self):
        result = self.verify({})
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("cannot read Nextest binary metadata", result.stderr)

    def test_host_binary_is_not_checked(self):
        metadata = self.metadata(self.static, self.dynamic)
        metadata["rust-binaries"]["1"]["build-platform"] = "host"
        result = self.verify(metadata)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_non_test_binary_is_checked(self):
        for binary, succeeds in [(self.static, True), (self.dynamic, False)]:
            with self.subTest(binary=binary):
                metadata = self.metadata()
                metadata["rust-build-meta"]["non-test-binaries"] = {
                    "fixture": [{
                        "kind": "bin-exe",
                        "path": str(binary.relative_to(self.root)),
                        "build-platform": "target",
                    }]
                }
                result = self.verify(metadata)
                self.assertEqual(result.returncode == 0, succeeds, result.stderr)

    def test_real_nextest_musl_metadata(self):
        project = self.root / "nextest fixture"
        (project / "src").mkdir(parents=True)
        (project / "Cargo.toml").write_text(
            '[package]\nname = "static-linking-fixture"\nversion = "0.1.0"\n'
            'edition = "2024"\n[workspace]\n', encoding="utf-8"
        )
        (project / "src" / "main.rs").write_text(
            'fn main() {}\n#[test]\nfn example() {}\n', encoding="utf-8"
        )
        (project / "tests").mkdir()
        (project / "tests" / "example.rs").write_text(
            '#[test]\nfn example() { let _ = env!("CARGO_BIN_EXE_static-linking-fixture"); }\n',
            encoding="utf-8",
        )
        metadata = subprocess.run(
            ["cargo", "nextest", "list", "--manifest-path", str(project / "Cargo.toml"),
             "--target", TARGET, "--target-dir", str(project / "output with spaces"),
             "--offline", "--list-type", "binaries-only", "--message-format", "json"],
            check=True, capture_output=True, text=True,
        )
        result = self.verify(json.loads(metadata.stdout))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Checked 3 target executables", result.stdout)


unittest.main()
PY
