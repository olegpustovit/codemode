"""Fault checks use an owned HOME and executable fixtures, never service files."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("access_release", Path(__file__).with_name("access_release.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="codemode-eval-release-")
        self.addCleanup(self.temporary.cleanup)
        self.home = Path(self.temporary.name)
        self.root = self.home / ".local/share/codemode"
        self.bin = self.root / "bin"
        self.bin.mkdir(parents=True)
        self.bundle(self.bin, "legacy")
        self.unit = self.home / ".config/systemd/user/smdctl-codemode.service"
        self.unit.parent.mkdir(parents=True)
        self.unit.write_text("[Service]\nExecStart=" + str(self.bin / "codemode") + "\n")
        self.env = self.home / ".config/smdctl/env/codemode.env"
        self.env.parent.mkdir(parents=True)
        self.env.write_text("CODEMODE_ID_ENABLED=true\nEVAL_SECRET=fixture\n")
        self.env.chmod(0o600)
        self.legacy = {"sha": "1" * 40, "schema": 22, "binary": module.digest(self.bin / "codemode"), "access": False}
        self.guard = module.ReleaseGuard(self.root, home=self.home, legacy=self.legacy)
        self.upload = self.home / "incoming"
        self.upload.mkdir(mode=0o700)
        self.bundle(self.upload, "candidate")

    def bundle(self, directory, label):
        for name in module.FILES:
            contents = (label + " " + name).encode()
            if name == "codemode":
                contents = ("#!/usr/bin/python3\nimport json,os,sys\nprint(json.dumps({'label':" + repr(label) + ", 'access':os.environ.get('CODEMODE_ID_ACCESS_ENABLED'),'worker':os.environ.get('CODEMODE_MONTY_BIN'),'args':sys.argv[1:],'secretPreserved':os.environ.get('EVAL_SECRET')=='fixture'}))\n").encode()
            module.write_private(directory / name, contents, 0o700)

    def install(self, sha="2" * 40, access=None):
        return self.guard.install(self.upload, sha, 22, access)

    def run_entry(self, inherited, *args):
        # No inherited credentials can appear in this fixture or its output.
        env = {"PATH": os.environ["PATH"], "HOME": str(self.home), "CODEMODE_ID_ACCESS_ENABLED": inherited, "CODEMODE_MONTY_BIN": "/wrong/worker", "EVAL_SECRET": "fixture"}
        return subprocess.run([self.bin / "codemode", *args], env=env, capture_output=True, text=True)

    def test_default_off_bundle_and_cli_mode_are_frozen_against_inherited_true(self):
        before = self.guard.protected()
        value = self.install()
        result = self.run_entry("true", "token", "create", "eval")
        self.assertEqual(result.returncode, 0, result.stderr)
        actual = json.loads(result.stdout)
        self.assertEqual(actual["access"], "false")
        self.assertEqual(actual["worker"], str(self.root / "releases" / value["id"] / "worker.sh"))
        self.assertEqual(actual["args"], ["token", "create", "eval"])
        self.assertTrue(actual["secretPreserved"])
        self.assertEqual(before, self.guard.protected())
        self.assertTrue(value["capable"])
        self.assertFalse(self.guard.metadata("previous")["capable"])

    def test_legacy_cannot_be_first_enabled_release(self):
        with self.assertRaisesRegex(RuntimeError, "default-off baseline"):
            self.install(access=True)
        self.assertIsNone(self.guard.metadata())
        self.assertEqual(module.digest(self.bin / "codemode"), self.legacy["binary"])

    def test_enabled_release_overrides_inherited_false_and_rollback_keeps_true(self):
        baseline = self.install()
        enabled = self.install(sha="3" * 40, access=True)
        actual = json.loads(self.run_entry("false").stdout)
        self.assertEqual(actual["access"], "true")
        previous, mode = self.guard.rollback_target()
        self.assertEqual(previous["id"], baseline["id"])
        self.assertTrue(mode)
        restored = self.guard.restore(enabled, previous, mode)
        self.assertTrue(restored["access"])
        self.assertEqual(json.loads(self.run_entry("false").stdout)["access"], "true")

    def test_unspecified_deploy_preserves_true_and_false_requires_explicit_mode(self):
        self.install()
        self.install(sha="3" * 40, access=True)
        implicit = self.install(sha="4" * 40)
        self.assertTrue(implicit["access"])
        off = self.install(sha="5" * 40, access=False)
        self.assertFalse(off["access"])
        previous, mode = self.guard.rollback_target()
        self.assertTrue(mode)
        self.guard.restore(off, previous, mode)
        self.assertEqual(json.loads(self.run_entry("false").stdout)["access"], "true")

    def test_rollback_to_legacy_refuses_any_enforced_selection(self):
        value = self.install()
        value["access"] = True
        self.guard.save(value)
        with self.assertRaisesRegex(RuntimeError, "preserve enforcement"):
            self.guard.rollback_target()

    def test_unknown_legacy_and_incomplete_workers_are_refused(self):
        for name in module.FILES:
            with self.subTest(name=name):
                path = self.bin / name
                data = path.read_bytes()
                path.unlink()
                with self.assertRaises(RuntimeError):
                    self.guard.current()
                module.write_private(path, data, 0o700)
        with self.assertRaises(RuntimeError):
            module.ReleaseGuard(self.root, home=self.home).current()

    def test_mode_requires_literal_boolean_and_full_sha_and_schema(self):
        for value in ("true", "false", 0, 1, [], {}):
            with self.subTest(value=value), self.assertRaises(ValueError):
                self.guard.select(value)
        for sha, schema in (("abc123", 22), ("x" * 40, 22), ("2" * 40, True), ("2" * 40, 0)):
            with self.subTest(sha=sha, schema=schema), self.assertRaises(ValueError):
                self.guard.bundle(self.upload, sha, schema, False, True)

    def test_stale_config_or_release_cannot_activate(self):
        self.install()
        token = self.guard.state()["token"]
        original = self.env.read_bytes()
        self.env.write_bytes(original + b"NEW_SETTING=reviewed\n")
        with self.assertRaises(RuntimeError):
            self.guard.expect(token)
        self.env.write_bytes(original)
        self.install(sha="3" * 40)
        with self.assertRaises(RuntimeError):
            self.guard.expect(token)

    def test_mode_metadata_corruption_fails_runtime_and_deploy_closed(self):
        value = self.install()
        for mode in ("false", "true", 0, 1, None, {}, []):
            with self.subTest(mode=mode):
                changed = dict(value, access=mode)
                self.guard.save(changed)
                self.assertNotEqual(self.run_entry("false").returncode, 0)
                with self.assertRaises(RuntimeError):
                    self.guard.current()
        self.guard.save(value)

    def test_each_worker_checksum_is_enforced_by_actual_dispatcher(self):
        value = self.install()
        for name in module.FILES:
            with self.subTest(name=name):
                path = self.root / "releases" / value["id"] / name
                original = path.read_bytes()
                path.write_bytes(original + b"changed")
                self.assertNotEqual(self.run_entry("false").returncode, 0)
                with self.assertRaises(RuntimeError):
                    self.guard.current()
                path.write_bytes(original)

    def test_private_metadata_artifacts_and_bundle_permissions_required(self):
        value = self.install()
        paths = [self.root / "current.release.json", self.bin / "codemode", self.root / "releases" / value["id"] / "monty"]
        for path in paths:
            with self.subTest(path=path.name):
                mode = path.stat().st_mode & 0o777
                path.chmod(0o755)
                self.assertNotEqual(self.run_entry("false").returncode, 0)
                with self.assertRaises(RuntimeError):
                    self.guard.current()
                path.chmod(mode)

    def test_same_sha_different_bundle_and_partial_upload_cannot_select(self):
        current = self.install()
        token = self.guard.state()["token"]
        (self.upload / "monty").write_bytes(b"different")
        with self.assertRaisesRegex(RuntimeError, "same SHA"):
            self.install()
        self.assertEqual(self.guard.state()["token"], token)
        (self.upload / "worker.bpf").unlink()
        with self.assertRaises(RuntimeError):
            self.install(sha="3" * 40)
        self.assertEqual(self.guard.current()["id"], current["id"])

    def test_validation_hashes_remain_authoritative_after_upload_changes(self):
        expected = {name: module.digest(self.upload / name) for name in module.FILES}
        (self.upload / "monty").write_bytes(b"changed after validation")
        with self.assertRaisesRegex(RuntimeError,"source changed"):
            self.guard.install(self.upload,"2"*40,22,False,expected)
        self.assertIsNone(self.guard.metadata())

    def test_custom_command_args_and_competing_dropins_are_refused(self):
        original = self.unit.read_text()
        for command in (str(self.bin / "codemode") + " --custom", "/other/codemode"):
            self.unit.write_text("[Service]\nExecStart=" + command + "\n")
            with self.assertRaises(RuntimeError):
                self.install()
        self.unit.write_text(original)
        self.guard.dropins.mkdir()
        (self.guard.dropins / "custom.conf").write_text("[Service]\nExecStart=/other/codemode\n")
        with self.assertRaises(RuntimeError):
            self.install()

    def test_existing_nested_backup_task_command_is_preserved(self):
        (self.root / "smdctl.yml").write_text("name: codemode\ncommand: " + str(self.bin / "codemode") + "\ntasks:\n  - name: backup\n    command: /owned/backup.sh\n    args: [daily]\n")
        before=self.guard.protected()
        self.install()
        self.assertEqual(self.guard.protected(),before)

    def test_failed_candidate_artifact_can_restore_intact_previous_bundle(self):
        self.install()
        selected = self.install(sha="3" * 40, access=True)
        previous, mode = self.guard.rollback_target()
        (self.root / "releases" / selected["id"] / "monty").write_bytes(b"injected corruption")
        self.assertNotEqual(self.run_entry("false").returncode, 0)
        self.guard.restore(selected, previous, mode)
        self.assertEqual(json.loads(self.run_entry("false").stdout)["access"], "true")

    def test_stale_rollback_cannot_replace_newer_selection(self):
        self.install()
        selected = self.install(sha="3" * 40, access=True)
        previous, mode = self.guard.rollback_target()
        newest = self.install(sha="4" * 40)
        with self.assertRaises(RuntimeError):
            self.guard.restore(selected, previous, mode)
        self.assertEqual(self.guard.current()["id"], newest["id"])

    def test_interrupted_initial_dispatcher_adoption_can_finish_only_known_legacy(self):
        baseline = self.guard.current()
        copied = self.guard.bundle(self.bin, baseline["sha"], baseline["schema"], False, False)
        self.guard.save(copied)
        self.assertEqual(self.guard.current()["id"], copied["id"])
        self.guard.expect(self.guard.state()["token"])
        selected = self.install()
        self.assertTrue(selected["capable"])
        self.assertEqual(json.loads(self.run_entry("true").stdout)["access"], "false")


if __name__ == "__main__":
    unittest.main()
