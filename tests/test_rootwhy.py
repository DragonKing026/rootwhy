# SPDX-License-Identifier: GPL-3.0-or-later
"""Tests of the command model and explanations (no GTK, no sudo). Run: python3 -m unittest discover tests"""
import contextlib
import importlib.machinery
import importlib.util
import io
import os
import sys
import tempfile
import unittest
from pathlib import Path

os.environ["ROOTWHY_LANG"] = "en"
_path = Path(__file__).resolve().parent.parent / "rootwhy"
_loader = importlib.machinery.SourceFileLoader("rootwhy", str(_path))
_spec = importlib.util.spec_from_loader("rootwhy", _loader)
rw = importlib.util.module_from_spec(_spec)
sys.modules["rootwhy"] = rw
_loader.exec_module(rw)

INSTALL = ["install", "-m", "755", "-o", "root", "-g", "root", "/home/u/kod-po/nocne.sh", "/usr/local/bin/nocne.sh"]

MAN_SAMPLE = """INSTALL(1)                       User Commands                       INSTALL(1)

NAME
       install - copy files and set attributes

DESCRIPTION
       --backup[=CONTROL]
              make a backup of each existing destination file

       -b     like --backup but does not accept an argument

       -m, --mode=MODE
              set permission mode (as in chmod), instead of rwxr-xr-x

       -t, --target-directory=DIRECTORY
              copy all SOURCE arguments into DIRECTORY
"""


class ShellQuote(unittest.TestCase):
    def test_plain(self):
        self.assertEqual(rw.shell_quote("/usr/local/bin/x"), "/usr/local/bin/x")

    def test_spaces(self):
        self.assertEqual(rw.shell_quote("Moje Projekty"), "'Moje Projekty'")

    def test_invisible_characters_are_shown(self):
        self.assertEqual(rw.shell_quote("a\u200bb"), "$'a\\u200bb'")
        self.assertEqual(rw.shell_quote("a\nb"), "$'a\\nb'")
        self.assertIn("\\u202e", rw.shell_quote("evil\u202etxt.sh"))


class Tokenize(unittest.TestCase):
    def test_install_options_take_values(self):
        kinds = [(t.kind, t.arg) for t in rw.tokenize(INSTALL)]
        self.assertEqual(kinds[:3], [("command", "install"), ("option", "-m"), ("value", "755")])
        self.assertEqual(kinds[-2:], [("path", INSTALL[-2]), ("path", INSTALL[-1])])

    def test_role_destination(self):
        toks = rw.tokenize(INSTALL)
        self.assertEqual((toks[-2].role, toks[-1].role), ("", "destination"))

    def test_long_option_with_equals(self):
        toks = rw.tokenize(["install", "--mode=644", "a", "b"])
        self.assertEqual([(t.kind, t.shown, t.joined) for t in toks[1:3]],
                         [("option", "--mode=", False), ("value", "644", True)])
        self.assertEqual(toks[2].arg, "644")

    def test_attached_short_value(self):
        toks = rw.tokenize(["install", "-m755", "a", "b"])
        self.assertEqual((toks[2].kind, toks[2].arg), ("value", "755"))

    def test_subcommand(self):
        toks = rw.tokenize(["systemctl", "restart", "ollama"])
        self.assertEqual([t.kind for t in toks], ["command", "subcommand", "arg"])

    def test_end_of_options(self):
        toks = rw.tokenize(["rm", "--", "-weird"])
        self.assertEqual([t.kind for t in toks], ["command", "marker", "arg"])

    def test_value_options_from_man(self):
        toks = rw.tokenize(["foo", "-x", "val"], {"foo": {"-x"}})
        self.assertEqual(toks[2].kind, "value")


class Layout(unittest.TestCase):
    def test_short_stays_on_one_line(self):
        self.assertEqual(len(rw.layout(rw.tokenize(["systemctl", "restart", "ollama"]))), 1)

    def test_long_is_split_with_values_kept(self):
        lines = [rw.line_text(line) for line in rw.layout(rw.tokenize(INSTALL))]
        self.assertEqual(lines, ["install", "-m 755", "-o root", "-g root", INSTALL[-2], INSTALL[-1]])

    def test_joined_value(self):
        toks = rw.tokenize(["install", "--mode=644", "/a/very/long/path/" + "x" * 40, "/another/long/" + "y" * 30])
        self.assertIn("--mode=644", [rw.line_text(line) for line in rw.layout(toks)])


class Man(unittest.TestCase):
    def test_parse(self):
        summary, opts = rw.parse_man(MAN_SAMPLE)
        self.assertEqual(summary, "copy files and set attributes")
        self.assertEqual(opts["-m"][0], True)
        self.assertIn("permission mode", opts["--mode"][1])
        self.assertEqual(opts["--backup"][0], False)  # optional value does not consume the next argument
        self.assertIn("like --backup", opts["-b"][1])
        self.assertTrue(opts["-t"][0])

    def test_subcommand_entries_and_first_sentence(self):
        _summary, opts = rw.parse_man("""COMMANDS
       start PATTERN...
           Start (activate) one or more units specified on the command line. Note that unit
           names are matched.

       Paragraph text at the same indent
       continues here.
""")
        self.assertEqual(opts["start"][1], "Start (activate) one or more units specified on the command line")
        self.assertNotIn("Paragraph", opts)

    def test_overstrike_removed(self):
        summary, _ = rw.parse_man("NAME\n       f\bfo\boo - does f\bfoo\n")
        self.assertEqual(summary, "does foo")


class Explain(unittest.TestCase):
    def test_only_system_sources(self):
        rw._MAN_CACHE["fakecmd"] = ("does fake things", {"-l": (False, "long format"), "-a": (False, "all files")})
        toks = rw.tokenize(["fakecmd", "-la", "plain"])
        rw.explain(toks, use_system=True)
        self.assertEqual(toks[0].notes[0], "does fake things")
        self.assertEqual(toks[1].notes, ["-l  long format", "-a  all files"])
        self.assertEqual(toks[2].notes, [])                 # the system knows nothing → no tooltip
        self.assertEqual(rw.tooltip_markup(toks[2]), "")

    def test_command_shown_as_typed(self):
        ls = rw.shutil.which("ls", path=rw.SECURE_PATH)
        if ls and not rw._unsafe_for_root(ls):
            self.assertEqual(rw.tokenize([ls, "-la"])[0].shown, "ls")
        self.assertEqual(rw.tokenize(["/no/such/ls"])[0].shown, "/no/such/ls")

    def test_mode_and_owner_warnings(self):
        self.assertIn("setuid", rw.mode_warning("4755"))
        self.assertIsNone(rw.mode_warning("755"))
        self.assertIsNone(rw.mode_warning("u+x"))
        self.assertIsNone(rw.owner_warning("root:root"))
        self.assertIn("no such user", rw.owner_warning("no-such-user-xyz"))

    def test_path_warnings(self):
        with tempfile.TemporaryDirectory() as d:
            f = Path(d, "f.txt")
            f.write_text("x")
            self.assertEqual(rw.path_warnings(str(f)), [])                         # exists, nothing to say
            self.assertIn("will be replaced", rw.path_warnings(str(f), role="destination")[0])
            self.assertEqual(rw.path_warnings(str(Path(d, "new")), role="destination"),
                             ["does not exist yet (will be created)"])
            self.assertEqual(rw.path_warnings(str(Path(d, "new"))), ["⚠ does not exist"])
            self.assertIn("could be changed", rw.path_warnings(str(f), role="script")[0])

    def test_tooltip_escapes_markup(self):
        t = rw.Token("x", "arg", "x", notes=["a & <b>"])
        self.assertEqual(rw.tooltip_markup(t), "a &amp; &lt;b&gt;")


class Polish(unittest.TestCase):
    def test_translation(self):
        old = rw.LANG
        try:
            rw.LANG = "pl"
            self.assertEqual(rw._("Cancel"), "Anuluj")
            self.assertIn("zastąpiony", rw._("⚠ an existing file — it will be replaced"))
        finally:
            rw.LANG = old

    def test_catalog_placeholders_match(self):
        import re
        for en, pl in rw.PL.items():
            self.assertEqual(sorted(re.findall(r"\{\w+\}", en)), sorted(re.findall(r"\{\w+\}", pl)), en)


class Security(unittest.TestCase):
    def test_sudo_cmdline_exact_form_only(self):
        self.assertEqual(rw.parse_sudo_cmdline(b"/usr/bin/sudo\0-k\0-A\0--\0/usr/bin/true\0"), ["/usr/bin/true"])
        self.assertEqual(rw.parse_sudo_cmdline(b"sudo\0-k\0-A\0--\0/bin/rm\0-rf\0/x\0"), ["/bin/rm", "-rf", "/x"])
        self.assertIsNone(rw.parse_sudo_cmdline(b"sudo\0-k\0-A\0--\0rm\0/x\0"))       # bare name
        self.assertIsNone(rw.parse_sudo_cmdline(b"sudo\0-k\0-A\0--\0/tmp/l/../bin/rm\0"))  # not normalised
        self.assertIsNone(rw.parse_sudo_cmdline(b"sudo\0-A\0--\0true\0"))         # without -k
        self.assertIsNone(rw.parse_sudo_cmdline(b"sudo\0-k\0-A\0--\0"))           # no command
        self.assertIsNone(rw.parse_sudo_cmdline(b"/tmp/fake\0-k\0-A\0--\0true\0"))  # not sudo

    def test_fake_parent_is_rejected(self):
        self.assertIsNone(rw.sudo_parent_command(os.getpid()))  # comm is python, euid not 0

    def test_unsafe_for_root(self):
        self.assertFalse(rw._unsafe_for_root("/usr/bin/true"))
        with tempfile.TemporaryDirectory() as d:
            f = Path(d, "s.sh")
            f.write_text("#!/bin/sh\n")
            self.assertTrue(rw._unsafe_for_root(str(f)))           # owned by this user
        self.assertTrue(rw._unsafe_for_root("/nonexistent/xyz"))   # errors count as unsafe
        self.assertTrue(rw._unsafe_for_root("/tmp/../usr/bin/true"))  # ".." counts as unsafe
        with tempfile.TemporaryDirectory() as d:
            link = Path(d, "true")
            link.symlink_to("/usr/bin/true")
            self.assertTrue(rw._unsafe_for_root(str(link)))        # root-owned target, but a link anyone here can swap

    def test_safe_environ(self):
        saved = dict(os.environ)
        try:
            os.environ.update(PYTHONPATH="/evil", GIO_EXTRA_MODULES="/evil", LD_PRELOAD="/evil.so", LANG="pl_PL.UTF-8",
                              ROOTWHY_REASON="x", LC_TIME="C")
            rw._safe_environ()
            for k in ("PYTHONPATH", "GIO_EXTRA_MODULES", "LD_PRELOAD"):
                self.assertNotIn(k, os.environ)
            self.assertEqual((os.environ["LANG"], os.environ["ROOTWHY_REASON"], os.environ["LC_TIME"]),
                             ("pl_PL.UTF-8", "x", "C"))
            self.assertEqual(os.environ["PATH"], rw.SECURE_PATH)
        finally:
            os.environ.clear()
            os.environ.update(saved)


class Release(unittest.TestCase):
    def test_embedded_logo_matches_file(self):
        self.assertEqual(rw.LOGO_SVG, (_path.parent / "logo.svg").read_text())


class Cli(unittest.TestCase):
    def test_reason_required(self):
        self.assertEqual(rw.main_mode(["true"]), 2)

    def test_explain_option_removed(self):  # 2.0: descriptions come only from the system
        with self.assertRaises(SystemExit), contextlib.redirect_stderr(io.StringIO()):
            rw.main_mode(["-r", "a valid reason here", "-e", "true=x", "true"])

    def test_unknown_command(self):
        os.environ.setdefault("DISPLAY", ":0")
        self.assertEqual(rw.main_mode(["-r", "a valid reason here", "no-such-command-xyz"]), 127)

    def test_askpass_refuses_without_sudo_parent(self):
        self.assertEqual(rw.askpass_mode(), 1)


if __name__ == "__main__":
    unittest.main()
