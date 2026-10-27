# SPDX-License-Identifier: GPL-3.0-or-later
"""Tests of the plugin hook that stops bare sudo. Run: python3 -m unittest discover tests"""
import json
import subprocess
import sys
import unittest
from pathlib import Path

HOOK = Path(__file__).resolve().parent.parent / "hooks" / "no-bare-sudo.py"


def run(command: str) -> int:
    data = json.dumps({"tool_name": "Bash", "tool_input": {"command": command}})
    return subprocess.run([sys.executable, "-I", str(HOOK)], input=data, capture_output=True, text=True,
                          check=False).returncode


class Hook(unittest.TestCase):
    def test_blocks_command_words(self):
        for cmd in ("sudo dnf install htop", "cd /tmp && sudo rm x", "ls | sudo tee /etc/x", "FOO=1 sudo true",
                    "/usr/bin/sudo -k true", "pkexec true", "echo $(sudo cat /etc/shadow)", "nohup sudo true &",
                    "doas true", "su -c id", "run0 true", "x=1\nsudo true"):
            self.assertEqual(run(cmd), 2, cmd)

    def test_allows_mentions(self):
        for cmd in ('git commit -m "never call sudo directly"', "grep -rn sudo README.md", "rootwhy -r 'Why x.' -- dnf up",
                    "cat <<EOF > notes.txt\nsudo is blocked\nEOF\necho ok", "man sudo", "echo 'a; sudo b'",
                    "jako-root -o 'Opis zmiany.' true", "pseudo-cmd", "visudo -c"):
            self.assertEqual(run(cmd), 0, cmd)

    def test_bad_input_passes(self):
        self.assertEqual(subprocess.run([sys.executable, str(HOOK)], input="not json", capture_output=True, text=True,
                                        check=False).returncode, 0)


if __name__ == "__main__":
    unittest.main()
