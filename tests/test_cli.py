"""Cross-platform integration/regression tests. Never use real credentials or endpoints.

Usage: python3 tests/test_cli.py [path-to-howdo]
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import unittest
from pathlib import Path

from mock_server import create_server

BINARY = Path(
    sys.argv.pop(1)
    if len(sys.argv) > 1 and not sys.argv[1].startswith("-")
    else "target/release/howdo" + (".exe" if os.name == "nt" else "")
).resolve()


class CliTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if not BINARY.is_file():
            raise RuntimeError(f"Build howdo before testing: {BINARY}")
        cls.server = create_server()
        cls.server_thread = threading.Thread(target=cls.server.serve_forever, daemon=True)
        cls.server_thread.start()
        cls.base_url = f"http://127.0.0.1:{cls.server.server_port}"

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.server.server_close()
        cls.server_thread.join()

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="howdo-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.config_path = self.root / "howdo" / "config.json"
        self.config_path.parent.mkdir()
        self.env = dict(os.environ)
        for key in [
            "OPENAI_API_KEY",
            "AZURE_OPENAI_API_KEY",
            "ANTHROPIC_API_KEY",
            "CUSTOM_TEST_KEY",
            "HOWDO_PROFILE",
            "HOWDO_SHELL",
        ]:
            self.env.pop(key, None)
        self.env.update(XDG_CONFIG_HOME=str(self.root), SHELL="/bin/sh", TERM="xterm")
        self.server.state = {"requests": []}
        self.config = {"provider": "local", "base_url": self.base_url + "/v1", "model": "default"}
        self.save_config()

    def save_config(self, config=None):
        self.config_path.write_text(json.dumps(config or self.config), encoding="utf-8")

    def run_cli(self, *args, input="", timeout=10):
        return subprocess.run(
            [str(BINARY), *args],
            input=input,
            text=True,
            encoding="utf-8",
            capture_output=True,
            env=self.env,
            timeout=timeout,
        )

    @property
    def requests(self):
        return self.server.state["requests"]

    def test_help_version_and_no_args(self):
        self.assertIn("Natural language", self.run_cli("--help").stdout)
        self.assertRegex(self.run_cli("--version").stdout, r"^howdo \d+\.\d+\.\d+")
        result = self.run_cli()
        self.assertEqual(result.returncode, 2)
        self.assertIn("Usage", result.stderr)
        self.assertEqual(self.requests, [])

    def test_eof_blank_and_piped_yes_never_execute(self):
        self.server.state["command"] = "echo HOWDO_EXECUTED"
        for input in ["", "\n", "y\n"]:
            with self.subTest(input=input):
                result = self.run_cli("print some text", input=input)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout, "echo HOWDO_EXECUTED\n")
                self.assertNotIn("Run?", result.stderr)

    def test_print_json_and_explicit_execution(self):
        self.server.state["command"] = "echo HOWDO_EXECUTED"
        result = self.run_cli("--print", "test")
        self.assertEqual(result.stdout, "echo HOWDO_EXECUTED\n")
        self.assertEqual(result.stderr, "")
        result = self.run_cli("--json", "test")
        data = json.loads(result.stdout)
        self.assertEqual(data["command"], "echo HOWDO_EXECUTED")
        self.assertEqual(data["profile"], "default")
        self.assertEqual(data["risks"], [])
        result = self.run_cli("--yes", "test")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), "HOWDO_EXECUTED")

    def test_execution_status_and_launch_error(self):
        self.server.state["command"] = "exit /b 7" if os.name == "nt" else "exit 7"
        result = self.run_cli("--yes", "test")
        self.assertEqual(result.returncode, 7, result.stderr)
        self.assertIn("status 7", result.stderr)
        missing = str(self.root / ("cmd.exe" if os.name == "nt" else "bash"))
        result = self.run_cli("--shell", missing, "--yes", "test")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Failed to execute", result.stderr)

    def test_risk_requires_explicit_override(self):
        # A harmless echo exercises the conservative warning without deleting anything.
        self.server.state["command"] = 'echo "rm -fr example"'
        result = self.run_cli("--yes", "test")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(result.stdout, "")
        self.assertIn("--allow-risky", result.stderr)
        result = self.run_cli("--yes", "--allow-risky", "test")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("rm -fr example", result.stdout)

    def test_local_and_other_do_not_receive_openai_key(self):
        self.env["OPENAI_API_KEY"] = "GLOBAL_OPENAI_TEST_KEY"
        self.assertEqual(self.run_cli("--print", "test").returncode, 0)
        self.assertNotIn("authorization", self.requests[-1]["headers"])
        self.config.update(provider="other", model="model", api_key="OTHER_TEST_KEY")
        self.save_config()
        self.assertEqual(self.run_cli("--print", "test").returncode, 0)
        self.assertEqual(self.requests[-1]["headers"]["authorization"], "Bearer OTHER_TEST_KEY")

    def test_explicit_credential_environment_variable(self):
        self.config.update(api_key_env="CUSTOM_TEST_KEY")
        self.save_config()
        result = self.run_cli("--print", "test")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.requests, [])
        self.env["CUSTOM_TEST_KEY"] = "CUSTOM_SECRET"
        self.assertEqual(self.run_cli("--print", "test").returncode, 0)
        self.assertEqual(self.requests[-1]["headers"]["authorization"], "Bearer CUSTOM_SECRET")

    def test_openai_endpoint_auth_and_reasoning_payload(self):
        self.config.update(provider="openai", model="gpt-5.2", api_key="SAVED_TEST_KEY")
        self.save_config()
        self.env["OPENAI_API_KEY"] = "ENV_TEST_KEY"
        result = self.run_cli("--print", "test query")
        self.assertEqual(result.returncode, 0, result.stderr)
        request = self.requests[-1]
        self.assertEqual(request["path"], "/v1/chat/completions")
        self.assertEqual(request["headers"]["authorization"], "Bearer ENV_TEST_KEY")
        self.assertEqual(request["body"]["model"], "gpt-5.2")
        self.assertEqual(request["body"]["messages"][0]["role"], "developer")
        self.assertEqual(request["body"]["messages"][1]["content"], "test query")
        self.assertEqual(request["body"]["max_completion_tokens"], 1024)
        self.assertNotIn("temperature", request["body"])

    def test_azure_full_endpoint_and_auth(self):
        endpoint = (
            self.base_url
            + "/openai/deployments/test/chat/completions?api-version=2024-12-01-preview"
        )
        self.config.update(
            provider="azure_openai", base_url=endpoint, model="", api_key="AZURE_TEST_KEY"
        )
        self.save_config()
        result = self.run_cli("--print", "test")
        self.assertEqual(result.returncode, 0, result.stderr)
        request = self.requests[-1]
        self.assertEqual(request["path"], endpoint.removeprefix(self.base_url))
        self.assertEqual(request["headers"]["api-key"], "AZURE_TEST_KEY")
        self.assertNotIn("authorization", request["headers"])
        self.assertNotIn("model", request["body"])

    def test_anthropic_endpoint_auth_and_mixed_content(self):
        self.config.update(
            provider="anthropic",
            base_url=self.base_url,
            model="claude-test",
            api_key="ANTHROPIC_TEST_KEY",
        )
        self.save_config()
        self.server.state["response"] = {
            "content": [
                {"type": "thinking", "thinking": "reasoning"},
                {"type": "text", "text": "echo mixed-ok"},
            ],
            "stop_reason": "end_turn",
        }
        result = self.run_cli("--print", "test")
        self.assertEqual(result.stdout, "echo mixed-ok\n", result.stderr)
        request = self.requests[-1]
        self.assertEqual(request["path"], "/v1/messages")
        self.assertEqual(request["headers"]["x-api-key"], "ANTHROPIC_TEST_KEY")
        self.assertEqual(request["headers"]["anthropic-version"], "2023-06-01")
        self.assertIn("system", request["body"])
        self.assertEqual(request["body"]["messages"][0]["role"], "user")

    def test_explicit_request_options(self):
        self.config.update(
            provider="other",
            model="custom",
            request_options={
                "token_limit": "max_completion_tokens",
                "max_tokens": 2048,
                "temperature": 0.0,
                "system_role": "developer",
                "reasoning_effort": "none",
            },
        )
        self.save_config()
        result = self.run_cli("--print", "test")
        self.assertEqual(result.returncode, 0, result.stderr)
        body = self.requests[-1]["body"]
        self.assertEqual(body["max_completion_tokens"], 2048)
        self.assertEqual(body["temperature"], 0.0)
        self.assertEqual(body["reasoning_effort"], "none")
        self.assertEqual(body["messages"][0]["role"], "developer")

    def test_invalid_responses_fail_without_panic_or_execution(self):
        for command in [
            "`",
            "```",
            "",
            "echo 'alpha\nbeta'",
            "echo \\\n done",
            "echo ok\x1b[2J",
            "echo \u202ebad",
            "echo ok</s>",
            "a" * 16385,
        ]:
            with self.subTest(command=command[:40]):
                self.server.state["command"] = command
                result = self.run_cli("--yes", "test")
                self.assertNotEqual(result.returncode, 0)
                self.assertNotEqual(result.returncode, 101)
                self.assertEqual(result.stdout, "")
                self.assertNotIn("panicked", result.stderr)
                self.assertNotIn("\x1b", result.stderr)

    def test_valid_fence_and_shell_text_are_preserved(self):
        self.server.state["command"] = "```sh\necho '# a ; b'\n```"
        result = self.run_cli("--print", "test")
        self.assertEqual(result.stdout, "echo '# a ; b'\n", result.stderr)

    def test_incomplete_refused_and_empty_choices_fail(self):
        for response in [
            {"choices": []},
            {"choices": [{"finish_reason": "length", "message": {"content": "echo partial"}}]},
            {"choices": [{"finish_reason": "tool_calls", "message": {"content": "echo partial"}}]},
            {"choices": [{"message": {"content": None, "refusal": "No"}}]},
            b"not-json",
        ]:
            with self.subTest(response=response):
                self.server.state["response"] = response
                result = self.run_cli("--yes", "test")
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(result.stdout, "")

    def test_http_errors_are_nonzero_sanitized_and_redacted(self):
        self.config["api_key"] = "PRIVATE_TEST_KEY"
        self.save_config()
        self.server.state.update(status=401, response=b"PRIVATE_TEST_KEY\x1b[2J failed")
        result = self.run_cli("--print", "test")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("401", result.stderr)
        self.assertIn("[redacted]", result.stderr)
        self.assertNotIn("PRIVATE_TEST_KEY", result.stderr)
        self.assertNotIn("\x1b", result.stderr)

    def test_structured_error_credentials_are_decoded_before_redaction(self):
        key = 'TEST_KEY_WITH_"QUOTES'
        self.config["api_key"] = key
        self.save_config()
        self.server.state.update(
            status=401, response={"error": {"message": f"Rejected key: {key}"}}
        )
        result = self.run_cli("--print", "test")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("[redacted]", result.stderr)
        self.assertNotIn("TEST_KEY_WITH", result.stderr)

    def test_redirects_are_not_followed(self):
        self.server.state.update(status=307, headers={"Location": self.base_url + "/redirect"})
        result = self.run_cli("--print", "test")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Redirects are disabled", result.stderr)
        self.assertEqual(len(self.requests), 1)

    def test_response_limits_with_and_without_content_length(self):
        for extra in [{}, {"omit_length": True}, {"chunked": True}]:
            with self.subTest(extra=extra):
                self.server.state = {"requests": [], "response": b"x" * (1024 * 1024 + 1), **extra}
                result = self.run_cli("--print", "test")
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("limit", result.stderr)

    def test_truncated_http_body_is_rejected(self):
        self.server.state["advertised_length"] = 10000
        result = self.run_cli("--yes", "test")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("truncated", result.stderr)
        self.assertEqual(result.stdout, "")

    def test_timeout_is_an_error(self):
        self.config["timeout_seconds"] = 1
        self.save_config()
        self.server.state["delay"] = 2
        result = self.run_cli("--print", "test")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("HTTP", result.stderr)

    def test_broken_and_missing_configs_never_launch_wizard(self):
        original = b"{broken"
        self.config_path.write_bytes(original)
        for args in [("test",), ("/config",)]:
            result = self.run_cli(*args)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(self.config_path.read_bytes(), original)
            self.assertNotIn("Provider (1-5)", result.stderr)
        self.config_path.unlink()
        result = self.run_cli("test")
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.config_path.exists())
        self.assertEqual(self.requests, [])

    def test_invalid_configuration_is_rejected_before_network(self):
        for change in [
            {"provider": "typo"},
            {"timeout_seconds": 0},
            {"api_key": "key\r\nheader"},
            {"base_url": "https://user:key@example.com/v1"},
            {"base_url": "http://example.com/v1"},
            {"request_options": {"max_tokens": 0}},
            {"unknown_field": True},
        ]:
            with self.subTest(change=change):
                self.save_config({**self.config, **change})
                self.assertNotEqual(self.run_cli("test").returncode, 0)
                self.assertEqual(self.requests, [])

    def test_profiles_switching_and_environment_selection(self):
        config = {
            "schema_version": 1,
            "default_profile": "default",
            "profiles": {
                "default": self.config,
                "work": {**self.config, "model": "work-model"},
            },
        }
        self.save_config(config)
        result = self.run_cli("/profiles")
        self.assertIn("* default", result.stdout)
        self.assertIn("work", result.stdout)
        self.assertEqual(self.run_cli("--profile", "work", "--print", "test").returncode, 0)
        self.assertEqual(self.requests[-1]["body"]["model"], "work-model")
        self.env["HOWDO_PROFILE"] = "work"
        self.assertEqual(self.run_cli("--print", "test").returncode, 0)
        self.assertEqual(self.requests[-1]["body"]["model"], "work-model")
        self.assertEqual(self.run_cli("/profiles", "use", "work").returncode, 0)
        self.assertEqual(json.loads(self.config_path.read_text())["default_profile"], "work")
        before = self.config_path.read_bytes()
        self.assertNotEqual(self.run_cli("/profiles", "use", "missing").returncode, 0)
        self.assertEqual(before, self.config_path.read_bytes())

    def test_explanations_do_not_change_or_execute_commands(self):
        self.server.state["command"] = "echo HOWDO_EXECUTED"
        result = self.run_cli("--explain", "--print", "test")
        self.assertEqual(result.stdout, "echo HOWDO_EXECUTED\n", result.stderr)
        self.assertIn("model-generated", result.stderr)
        self.assertEqual(len(self.requests), 2)
        self.assertTrue(
            self.requests[1]["body"]["messages"][-1]["content"].startswith("Explain this command:")
        )
        result = self.run_cli("--explain", "--json", "test")
        self.assertIn("Prints text", json.loads(result.stdout)["explanation"])
        self.assertEqual(result.stderr, "")

    def test_options_conflicts_fail_before_network(self):
        for args in [
            ("--print", "--yes", "test"),
            ("--copy", "--json", "test"),
            ("--allow-risky", "test"),
            ("--shell",),
            ("--unknown", "test"),
        ]:
            self.assertNotEqual(self.run_cli(*args).returncode, 0)
        self.assertEqual(self.requests, [])

    def test_update_requires_explicit_noninteractive_authorization(self):
        result = self.run_cli("/update")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("--yes", result.stderr)

    @unittest.skipIf(os.name == "nt", "Unix clipboard stub")
    def test_clipboard_uses_stdin_and_never_executes(self):
        directory = self.root / "bin"
        directory.mkdir()
        captured = self.root / "clipboard.txt"
        self.env.update(
            PATH=str(directory) + os.pathsep + self.env.get("PATH", ""),
            WAYLAND_DISPLAY="howdo-test",
            HOWDO_CLIPBOARD_TEST_FILE=str(captured),
        )
        name = "pbcopy" if sys.platform == "darwin" else "wl-copy"
        stub = directory / name
        stub.write_text('#!/bin/sh\ncat > "$HOWDO_CLIPBOARD_TEST_FILE"\n', encoding="utf-8")
        stub.chmod(0o755)
        self.server.state["command"] = "echo HOWDO_EXECUTED"
        result = self.run_cli("--copy", "test")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "")
        self.assertEqual(captured.read_text(), "echo HOWDO_EXECUTED")

    @unittest.skipUnless(os.name == "nt", "Windows shell execution")
    def test_actual_windows_powershell_and_cmd(self):
        for executable in ["powershell", "pwsh", "cmd.exe"]:
            if not shutil.which(executable):
                continue
            with self.subTest(executable=executable):
                self.server.state["command"] = "echo HOWDO_EXECUTED"
                result = self.run_cli("--shell", executable, "--yes", "test")
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn("HOWDO_EXECUTED", result.stdout)
                self.server.state["command"] = "exit /b 7" if executable == "cmd.exe" else "exit 7"
                self.assertEqual(self.run_cli("--shell", executable, "--yes", "test").returncode, 7)

    @unittest.skipUnless(os.name == "nt", "Windows shell quoting")
    def test_windows_shell_preserves_quotes_unicode_and_native_status(self):
        target = self.root / "quoted output.txt"
        self.server.state["command"] = f'echo "HOWDO QUOTED">"{target}" & exit /b 7'
        result = self.run_cli("--shell", "cmd.exe", "--yes", "test")
        self.assertEqual(result.returncode, 7, result.stderr)
        self.assertEqual(target.read_text().strip(), '"HOWDO QUOTED"')
        data = 'café "quoted" & <tag> \\ path'
        for executable in ["powershell", "pwsh"]:
            if not shutil.which(executable):
                continue
            with self.subTest(executable=executable):
                path = str(target).replace("'", "''")
                self.server.state["command"] = (
                    f"[System.IO.File]::WriteAllText('{path}', '{data}', [System.Text.UTF8Encoding]::new()) # comment"
                )
                result = self.run_cli("--shell", executable, "--yes", "test")
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(target.read_text(encoding="utf-8"), data)
                self.server.state["command"] = "cmd.exe /D /C 'exit 7' # comment"
                self.assertEqual(self.run_cli("--shell", executable, "--yes", "test").returncode, 7)

    @unittest.skipIf(os.name == "nt", "Unix PTY")
    def test_interactive_confirmation_editing_and_cancellation(self):
        def interactive(steps, expected_code=0):
            return self.terminal(
                ["test"],
                [(prompt.decode(), answer.decode()) for prompt, answer in steps],
                expected_code,
            )

        self.server.state["command"] = "echo HOWDO_EXECUTED"
        output = interactive([(b"Run?", b"\n")])
        self.assertNotIn("\r\nHOWDO_EXECUTED\r\n", output)
        output = interactive([(b"Run?", b"y\n")])
        self.assertIn("\r\nHOWDO_EXECUTED\r\n", output)
        output = interactive(
            [
                (b"Run?", b"e\n"),
                (b"  > ", b"\x15echo HOWDO_EDITED; echo 'rm -rf fake'\n"),
                (b"Type RUN", b"\n"),
            ]
        )
        self.assertNotIn("\r\nHOWDO_EDITED\r\n", output)
        self.assertIn("deletes files", output)
        self.server.state["delay"] = 2
        interactive([(b"Esc/Ctrl+C", b"\x03")], expected_code=130)

    def terminal(self, args, steps, expected_code=0):
        driver = Path(__file__).with_name("pty_driver.py")
        result = subprocess.run(
            [sys.executable, str(driver)],
            input=json.dumps({"argv": [str(BINARY), *args], "steps": steps}),
            text=True,
            capture_output=True,
            env=self.env,
            timeout=15,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        data = json.loads(result.stdout)
        self.assertEqual(data["code"], expected_code, data["output"])
        self.assertEqual(data["steps"], len(steps), data["output"])
        self.assertTrue(data["canonical"], "Terminal canonical mode was not restored")
        self.assertTrue(data["echo"], "Terminal echo was not restored")
        return data["output"]

    @unittest.skipIf(os.name == "nt", "Unix PTY typeahead")
    def test_keys_entered_during_generation_do_not_approve_a_future_command(self):
        # The reply arrives before the next progress timeout: this exercises the
        # completion/input race, not just the slower periodic input-drain path.
        self.server.state.update(delay=0.15, command="echo HOWDO_PREFILLED")
        output = self.terminal(["test"], [("Esc/Ctrl+C", "y\n"), ("Run?", "n\n")])
        self.assertNotIn("\r\nHOWDO_PREFILLED\r\n", output)
        self.env["TERM"] = "dumb"
        output = self.terminal(["test"], [("Esc/Ctrl+C", "y\n"), ("Run?", "n\n")])
        self.assertNotIn("\r\nHOWDO_PREFILLED\r\n", output)

    @unittest.skipIf(os.name == "nt", "Unix PTY environment credentials")
    def test_wizard_removes_saved_key_when_environment_credentials_are_selected(self):
        self.config["api_key"] = "OLD_SAVED_TEST_KEY"
        self.save_config()
        self.env["CUSTOM_TEST_KEY"] = "NEW_ENV_TEST_KEY"
        output = self.terminal(
            ["/config"],
            [
                ("Provider (1-5)", "1\n"),
                ("Endpoint URL", "\n"),
                ("Model", "\n"),
                ("Explicit credential environment variable", "CUSTOM_TEST_KEY\n"),
                ("Test authenticated inference", "y\n"),
            ],
        )
        store = json.loads(self.config_path.read_text())
        profile = store["profiles"]["default"]
        self.assertNotIn("api_key", profile)
        self.assertEqual(profile["api_key_env"], "CUSTOM_TEST_KEY")
        self.assertNotIn("OLD_SAVED_TEST_KEY", self.config_path.read_text())
        self.assertNotIn("NEW_ENV_TEST_KEY", output)
        self.assertEqual(self.requests[0]["headers"]["authorization"], "Bearer NEW_ENV_TEST_KEY")

    @unittest.skipIf(os.name == "nt", "Unix PTY wizard")
    def test_wizard_hides_credentials_and_tests_inference_without_execution(self):
        output = self.terminal(
            ["/config", "--profile", "new"],
            [
                ("Provider (1-5)", "1\n"),
                ("Endpoint URL", self.base_url + "/v1\n"),
                ("Model", "default\n"),
                ("Explicit credential environment variable", "\n"),
                ("API key (hidden", "HIDDEN_TEST_KEY\n"),
                ("Test authenticated inference", "y\n"),
            ],
        )
        self.assertNotIn("HIDDEN_TEST_KEY", output)
        store = json.loads(self.config_path.read_text())
        self.assertEqual(store["profiles"]["new"]["api_key"], "HIDDEN_TEST_KEY")
        self.assertEqual(self.requests[0]["headers"]["authorization"], "Bearer HIDDEN_TEST_KEY")
        self.assertIn("not executed", output)
        self.assertEqual(self.config_path.stat().st_mode & 0o777, 0o600)

    @unittest.skipIf(os.name == "nt", "Unix PTY wizard")
    def test_failed_wizard_inference_and_eof_preserve_configuration(self):
        before = self.config_path.read_bytes()
        self.server.state["status"] = 401
        self.terminal(
            ["/config"],
            [
                ("Provider (1-5)", "1\n"),
                ("Endpoint URL", "\n"),
                ("Model", "\n"),
                ("Explicit credential environment variable", "\n"),
                ("API key (hidden", "\n"),
                ("Test authenticated inference", "y\n"),
            ],
            expected_code=1,
        )
        self.assertEqual(self.config_path.read_bytes(), before)
        self.terminal(["/config"], [("Provider (1-5)", "\x04")], expected_code=130)
        self.assertEqual(self.config_path.read_bytes(), before)


if __name__ == "__main__":
    unittest.main(verbosity=2)
