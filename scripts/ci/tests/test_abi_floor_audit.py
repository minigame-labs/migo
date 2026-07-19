import importlib.util
import pathlib
import unittest

_MODULE_PATH = pathlib.Path(__file__).resolve().parents[1].parent / "abi-floor-audit.py"
_spec = importlib.util.spec_from_file_location("abi_floor_audit", _MODULE_PATH)
audit = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(audit)


# Captured verbatim from `objdump -T engine/target/debug/migo-player` (binutils
# 2.42). The version tag is parenthesised for undefined symbols; an earlier
# hand-written fixture omitted the parentheses, so the parser matched nothing on
# a real binary and the audit reported a clean pass. Fixtures for a gate must
# come from the tool the gate runs, never from memory.
OBJDUMP_T = """
DYNAMIC SYMBOL TABLE:
0000000000000000  w   D  *UND*\t0000000000000000              __gmon_start__
0000000000000000  w   DF *UND*\t0000000000000000 (GLIBC_2.2.5) __cxa_finalize
0000000000000000      DF *UND*\t0000000000000000 (GLIBC_2.2.5) memcmp
0000000000000000      DF *UND*\t0000000000000000 (GLIBC_2.38) __isoc23_strtol
0000000000000000      DF *UND*\t0000000000000000 (GLIBC_2.17) clock_gettime
0000000000000000      DF *UND*\t0000000000000000 (GLIBCXX_3.4.31) _ZSt28__throw_bad_array_new_lengthv
0000000000000000      DF *UND*\t0000000000000000 (CXXABI_1.3) __cxa_throw
0000000003d1a640 g    DF .text\t000000000000004a  Base        _Znwm
000000000004a1c0 g    DF .text\t0000000000000031  Base        migo_engine_create
"""

# Some objdump builds and some inputs print the tag without parentheses; the
# parser must accept both rather than silently reporting an empty result.
OBJDUMP_T_UNPARENTHESISED = """
DYNAMIC SYMBOL TABLE:
0000000000000000      DF *UND*\t0000000000000000  GLIBC_2.38  __isoc23_strtol
"""

# Captured from `objdump -T` on a real libmigo.so. The distinction that matters:
# a genuine export is defined in a real section (.text here); everything sitting
# in *ABS* is an import or a linker artifact, even though `nm --defined-only`
# reports it as defined. Reading nm instead of objdump made the export audit
# report 190 symbols for a library that exports 12.
OBJDUMP_T_EXPORTS = """
DYNAMIC SYMBOL TABLE:
0000000000055190 g    DF *ABS*\t00000000000000a8 (ALSA_0.9)   snd_pcm_open
00000000002307e5 g    DF .text\t00000000000002be  MIGO_1.0    migo_engine_create
0000000000230b00 g    DF .text\t0000000000000100  MIGO_1.0    migo_engine_destroy
0000000000009190  w   DF *ABS*\t000000000000005b (GLIBC_2.2.5) pthread_detach
00000000000460e0 g    DF *ABS*\t0000000000000000  Base        glActiveTexture
fffffffffc9cdb20 g    D  *ABS*\t0000000000000008  Base        _ZSt11__once_call
0000000000000000      DF *UND*\t0000000000000000 (GLIBC_2.17) clock_gettime
0000000000240000  w   DF .text\t0000000000000040  Base        rust_eh_personality
"""

OBJDUMP_P = """
Dynamic Section:
  NEEDED               libEGL.so.1
  NEEDED               libc.so.6
  SONAME               libmigo.so.1
"""


class ParseVersionNeedsTest(unittest.TestCase):
    def test_groups_versions_by_tag(self):
        needs = audit.parse_version_needs(OBJDUMP_T)
        self.assertEqual(needs["GLIBC"], {(2, 2, 5), (2, 38), (2, 17)})
        self.assertEqual(needs["GLIBCXX"], {(3, 4, 31)})
        self.assertEqual(needs["CXXABI"], {(1, 3)})

    def test_ignores_defined_and_unversioned_symbols(self):
        needs = audit.parse_version_needs(OBJDUMP_T)
        self.assertNotIn("Base", needs)

    def test_max_version_orders_numerically_not_lexically(self):
        needs = audit.parse_version_needs(OBJDUMP_T)
        # (2, 38) must beat (2, 2, 5); a string compare would pick "2.2.5".
        self.assertEqual(audit.max_version(needs, "GLIBC"), (2, 38))

    def test_max_version_of_absent_tag_is_none(self):
        self.assertIsNone(audit.max_version({}, "GLIBC"))

    def test_offending_symbols_reports_who_breaks_the_floor(self):
        offenders = audit.offending_symbols(OBJDUMP_T, "GLIBC", (2, 31))
        self.assertEqual(offenders, [("GLIBC_2.38", "__isoc23_strtol")])

    def test_accepts_the_unparenthesised_form(self):
        needs = audit.parse_version_needs(OBJDUMP_T_UNPARENTHESISED)
        self.assertEqual(needs["GLIBC"], {(2, 38)})


class ParseFailureIsNotAPassTest(unittest.TestCase):
    """A gate that can find nothing must say so rather than report success.

    Finding zero version records in output that has a dynamic symbol table means
    the parser did not understand the format -- exactly the failure that let a
    hand-written fixture mask a real 2.38 dependency.
    """

    def test_dynamic_table_with_no_parsed_versions_is_an_error(self):
        unparseable = "DYNAMIC SYMBOL TABLE:\n<some format we do not understand>\n"
        with self.assertRaises(audit.AuditParseError):
            audit.check_parse_sanity(unparseable, {})

    def test_a_table_with_parsed_versions_is_accepted(self):
        needs = audit.parse_version_needs(OBJDUMP_T)
        audit.check_parse_sanity(OBJDUMP_T, needs)  # must not raise

    def test_a_static_binary_with_no_dynamic_table_is_accepted(self):
        audit.check_parse_sanity("no dynamic symbol table here", {})  # must not raise


class FormatVersionTest(unittest.TestCase):
    def test_round_trips(self):
        self.assertEqual(audit.format_version((3, 4, 28)), "3.4.28")


class ParseExportedSymbolsTest(unittest.TestCase):
    def test_returns_symbols_defined_in_a_real_section(self):
        self.assertEqual(
            audit.parse_exported_symbols(OBJDUMP_T_EXPORTS),
            {"migo_engine_create", "migo_engine_destroy", "rust_eh_personality"},
        )

    def test_absolute_symbols_are_imports_not_exports(self):
        exported = audit.parse_exported_symbols(OBJDUMP_T_EXPORTS)
        for imported in ("snd_pcm_open", "pthread_detach", "glActiveTexture",
                         "_ZSt11__once_call"):
            self.assertNotIn(imported, exported)

    def test_undefined_symbols_are_not_exports(self):
        self.assertNotIn("clock_gettime", audit.parse_exported_symbols(OBJDUMP_T_EXPORTS))

    def test_weak_definitions_in_a_section_count(self):
        # A weak definition is still bindable by a host, so it belongs in the
        # audited surface even though it is not strong.
        self.assertIn("rust_eh_personality", audit.parse_exported_symbols(OBJDUMP_T_EXPORTS))


class ParseNeededTest(unittest.TestCase):
    def test_returns_needed_in_order(self):
        self.assertEqual(audit.parse_needed(OBJDUMP_P), ["libEGL.so.1", "libc.so.6"])

    def test_soname_is_not_a_dependency(self):
        self.assertNotIn("libmigo.so.1", audit.parse_needed(OBJDUMP_P))


if __name__ == "__main__":
    unittest.main()
