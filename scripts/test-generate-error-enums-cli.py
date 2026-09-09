#!/usr/bin/env python3
"""CLI behavior tests for generate-error-enums.py."""

from __future__ import annotations

import importlib.util
import tempfile
import unittest
from pathlib import Path
from unittest import mock

SCRIPT_PATH = Path(__file__).with_name("generate-error-enums.py")
SPEC = importlib.util.spec_from_file_location("generate_error_enums", SCRIPT_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError(f"could not load {SCRIPT_PATH}")
GENERATOR = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(GENERATOR)


class GeneratorCliTests(unittest.TestCase):
    def test_help_exits_before_fetch_or_write(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "output.json"
            with (
                mock.patch.object(GENERATOR, "OUTPUT_PATH", output),
                mock.patch.object(GENERATOR, "fetch_source") as fetch,
                self.assertRaises(SystemExit) as raised,
            ):
                GENERATOR.main(["--help"])
            self.assertEqual(raised.exception.code, 0)
            fetch.assert_not_called()
            self.assertFalse(output.exists())

    def test_unknown_and_abbreviated_flags_fail_before_fetch_or_write(self) -> None:
        for flag in ["--c", "--" + "che" + "k"]:
            with self.subTest(flag=flag), tempfile.TemporaryDirectory() as directory:
                output = Path(directory) / "output.json"
                with (
                    mock.patch.object(GENERATOR, "OUTPUT_PATH", output),
                    mock.patch.object(GENERATOR, "fetch_source") as fetch,
                    self.assertRaises(SystemExit) as raised,
                ):
                    GENERATOR.main([flag])
                self.assertEqual(raised.exception.code, 2)
                fetch.assert_not_called()
                self.assertFalse(output.exists())

    def test_default_regeneration_and_check_modes(self) -> None:
        messages = {f"MESSAGE_{index}": "message" for index in range(1_723)}
        openapi_codes = {f"SAFE_{index}" for index in range(56)}
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "output.json"
            patches = (
                mock.patch.object(GENERATOR, "OUTPUT_PATH", output),
                mock.patch.object(GENERATOR, "fetch_source", return_value="source"),
                mock.patch.object(GENERATOR, "parse_enums", return_value=messages),
                mock.patch.object(
                    GENERATOR, "load_openapi_codes", return_value=openapi_codes
                ),
                mock.patch.object(GENERATOR, "build_output", return_value="generated\n"),
            )
            with patches[0], patches[1], patches[2], patches[3], patches[4]:
                GENERATOR.main([])
                self.assertEqual(output.read_text(encoding="utf-8"), "generated\n")
                GENERATOR.main(["--check"])
                self.assertEqual(output.read_text(encoding="utf-8"), "generated\n")


if __name__ == "__main__":
    unittest.main()
