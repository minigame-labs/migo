import importlib.util
import os
import pathlib
import sys
import tempfile
import textwrap
import unittest

_MODULE_PATH = (
    pathlib.Path(__file__).resolve().parents[2] / "lib" / "verification_targets.py"
)
_spec = importlib.util.spec_from_file_location("verification_targets", _MODULE_PATH)
targets = importlib.util.module_from_spec(_spec)
# Registered before execution because `dataclasses` resolves a field's type through
# `sys.modules[cls.__module__]`, which is absent for a module loaded only by spec.
sys.modules[_spec.name] = targets
_spec.loader.exec_module(targets)


def write(root, relative, body=""):
    path = pathlib.Path(root) / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(textwrap.dedent(body))
    return path


class ConditionParsingTest(unittest.TestCase):
    """Which platforms a `cfg(...)` condition can select.

    The two cases a naive reader gets wrong: OpenHarmony is `target_os = "linux"`
    with `target_env = "ohos"`, so a rule keyed on `target_os` alone sees a host
    condition; and `cfg(windows)` names no key at all.
    """

    def test_host_only_conditions_select_nothing(self):
        self.assertEqual(targets.platforms_in('target_os = "linux"'), frozenset())
        self.assertEqual(targets.platforms_in("unix"), frozenset())
        self.assertEqual(targets.platforms_in("test"), frozenset())

    def test_android_is_selected_by_target_os(self):
        self.assertEqual(
            targets.platforms_in('target_os = "android"'), frozenset({"android"})
        )

    def test_ohos_is_selected_by_target_env_under_a_linux_target_os(self):
        self.assertEqual(
            targets.platforms_in('all(target_os = "linux", target_env = "ohos")'),
            frozenset({"ohos"}),
        )

    def test_a_platform_is_named_whichever_way_the_condition_points(self):
        # Polarity is deliberately ignored. Editing a block compiled *away* from a
        # platform cannot break that platform's compile by itself, but removing
        # something its sibling branch referenced can, and only that target sees it.
        self.assertEqual(
            targets.platforms_in('all(target_os = "linux", not(target_env = "ohos"))'),
            frozenset({"ohos"}),
        )
        self.assertEqual(targets.platforms_in("not(windows)"), frozenset({"windows"}))

    def test_bare_windows_is_selected_without_a_key(self):
        self.assertEqual(targets.platforms_in("windows"), frozenset({"windows"}))

    def test_a_test_escape_does_not_absorb_the_platform(self):
        # `any(target_os = "android", test)` compiles on the host under `cargo
        # test`, which is why this is worth a case: a host test run is not
        # evidence about the Android variant of the same module.
        self.assertEqual(
            targets.platforms_in('any(target_os = "android", test)'),
            frozenset({"android"}),
        )

    def test_an_unrecognised_target_os_is_reported_rather_than_dropped(self):
        self.assertEqual(
            targets.platforms_in('target_os = "macos"'), frozenset({"macos"})
        )


class ModuleResolutionTest(unittest.TestCase):
    """Every source file's inherited platform conditions.

    A file selected by a conditional need not contain one: `capi/src/platform/
    windows.rs` is plain Rust gated by its parent's `mod` declaration.
    """

    def resolve(self, root, crate="demo"):
        return targets.resolve_crate(pathlib.Path(root) / "engine/crates" / crate)

    def test_a_condition_on_a_mod_declaration_reaches_the_file_it_names(self):
        with tempfile.TemporaryDirectory() as root:
            write(
                root,
                "engine/crates/demo/src/lib.rs",
                """\
                #[cfg(target_os = "windows")]
                pub mod windows;
                """,
            )
            write(root, "engine/crates/demo/src/windows.rs", "pub fn f() {}\n")
            conditions, unreachable = self.resolve(root)
            self.assertEqual(unreachable, set())
            self.assertEqual(
                conditions["engine/crates/demo/src/windows.rs"],
                frozenset({"windows"}),
            )
            # The declaring file carries the condition too: editing the `cfg` line
            # itself is what decides whether the Windows build sees the module.
            self.assertEqual(
                conditions["engine/crates/demo/src/lib.rs"], frozenset({"windows"})
            )

    def test_a_condition_reaches_a_whole_subtree_through_unconditional_mods(self):
        with tempfile.TemporaryDirectory() as root:
            write(
                root,
                "engine/crates/demo/src/lib.rs",
                """\
                #[cfg(target_os = "android")]
                pub mod android;
                """,
            )
            write(root, "engine/crates/demo/src/android/mod.rs", "pub mod jni;\n")
            write(root, "engine/crates/demo/src/android/jni/mod.rs", "pub mod inbound;\n")
            write(
                root,
                "engine/crates/demo/src/android/jni/inbound.rs",
                "pub fn on_touch() {}\n",
            )
            conditions, unreachable = self.resolve(root)
            self.assertEqual(unreachable, set())
            self.assertEqual(
                conditions["engine/crates/demo/src/android/jni/inbound.rs"],
                frozenset({"android"}),
            )

    def test_a_path_attribute_redirects_the_condition_to_the_named_file(self):
        with tempfile.TemporaryDirectory() as root:
            write(
                root,
                "engine/crates/demo/src/lib.rs",
                """\
                #[path = "android/jni/profile_contract.rs"]
                pub(crate) mod jni_profile_contract;
                """,
            )
            write(
                root,
                "engine/crates/demo/src/android/jni/profile_contract.rs",
                "pub const SIG: &str = \"Z\";\n",
            )
            conditions, unreachable = self.resolve(root)
            self.assertEqual(unreachable, set())
            # Reached unconditionally through `#[path]`, so the host compiles it
            # and it needs no target build of its own.
            self.assertEqual(
                conditions["engine/crates/demo/src/android/jni/profile_contract.rs"],
                frozenset(),
            )

    def test_a_path_attribute_is_relative_to_the_declaring_files_directory(self):
        # The case that differs from a plain `mod`: a non-`mod.rs` parent. Rust
        # resolves `#[path]` against the directory the declaring file sits in, so
        # `#[path = "damage_tracker.rs"]` in src/dirty_region.rs names
        # src/damage_tracker.rs -- not src/dirty_region/damage_tracker.rs.
        with tempfile.TemporaryDirectory() as root:
            write(root, "engine/crates/demo/src/lib.rs", "pub mod dirty_region;\n")
            write(
                root,
                "engine/crates/demo/src/dirty_region.rs",
                """\
                #[cfg(target_os = "android")]
                #[path = "damage_tracker.rs"]
                pub mod damage_tracker;
                """,
            )
            write(root, "engine/crates/demo/src/damage_tracker.rs", "pub fn f() {}\n")
            conditions, unreachable = self.resolve(root)
            self.assertEqual(unreachable, set())
            self.assertEqual(
                conditions["engine/crates/demo/src/damage_tracker.rs"],
                frozenset({"android"}),
            )

    def test_a_files_own_inline_conditions_add_to_what_it_inherits(self):
        with tempfile.TemporaryDirectory() as root:
            write(root, "engine/crates/demo/src/lib.rs", "pub mod fs_ops;\n")
            write(
                root,
                "engine/crates/demo/src/fs_ops.rs",
                """\
                #[cfg(unix)]
                fn chmod() {}
                #[cfg(windows)]
                fn acl() {}
                """,
            )
            conditions, _ = self.resolve(root)
            self.assertEqual(
                conditions["engine/crates/demo/src/fs_ops.rs"],
                frozenset({"windows"}),
            )

    def test_conditions_from_two_declaration_sites_are_both_kept(self):
        with tempfile.TemporaryDirectory() as root:
            write(
                root,
                "engine/crates/demo/src/lib.rs",
                """\
                #[cfg(target_os = "android")]
                pub mod a;
                #[cfg(target_os = "windows")]
                #[path = "a.rs"]
                pub mod a_win;
                """,
            )
            write(root, "engine/crates/demo/src/a.rs", "pub fn f() {}\n")
            conditions, _ = self.resolve(root)
            self.assertEqual(
                conditions["engine/crates/demo/src/a.rs"],
                frozenset({"android", "windows"}),
            )

    def test_a_source_file_no_mod_declaration_reaches_is_reported_unreachable(self):
        # The parser's failure mode. An unreached file has unknown conditions, so
        # reporting it is what keeps a missed `mod` form from silently reading as
        # "this file needs no target build".
        with tempfile.TemporaryDirectory() as root:
            write(root, "engine/crates/demo/src/lib.rs", "pub mod known;\n")
            write(root, "engine/crates/demo/src/known.rs", "")
            write(root, "engine/crates/demo/src/orphan.rs", "")
            _, unreachable = self.resolve(root)
            self.assertEqual(unreachable, {"engine/crates/demo/src/orphan.rs"})

    def test_an_inline_module_body_does_not_claim_a_sibling_file(self):
        with tempfile.TemporaryDirectory() as root:
            write(
                root,
                "engine/crates/demo/src/lib.rs",
                """\
                pub mod tests {
                    pub fn t() {}
                }
                """,
            )
            write(root, "engine/crates/demo/src/tests.rs", "")
            _, unreachable = self.resolve(root)
            self.assertEqual(unreachable, {"engine/crates/demo/src/tests.rs"})


class SelectionTest(unittest.TestCase):
    """The plan a change produces."""

    def tree(self, root):
        write(
            root,
            "engine/crates/platform/src/lib.rs",
            """\
            #[cfg(target_os = "android")]
            pub mod android;
            """,
        )
        write(root, "engine/crates/platform/src/android/mod.rs", "pub mod jni;\n")
        write(root, "engine/crates/platform/src/android/jni.rs", "pub fn f() {}\n")
        write(root, "engine/crates/shared/src/lib.rs", "pub mod raf_signal;\n")
        write(
            root,
            "engine/crates/shared/src/raf_signal.rs",
            """\
            #[cfg(target_os = "android")]
            fn choreographer() {}
            """,
        )
        write(root, "engine/crates/io/src/lib.rs", "pub mod atomic_write;\n")
        write(
            root,
            "engine/crates/io/src/atomic_write.rs",
            """\
            #[cfg(windows)]
            fn replace() {}
            """,
        )
        write(root, "engine/crates/core/src/lib.rs", "pub mod runtime;\n")
        write(root, "engine/crates/core/src/runtime.rs", "pub fn f() {}\n")
        write(root, "engine/crates/android-jni/src/lib.rs", "pub fn f() {}\n")
        write(root, "docs/note.md", "text\n")
        return pathlib.Path(root)

    def plan(self, root, *changed):
        return targets.select(self.tree(root), changed)

    def platforms(self, plan):
        return {r.platform: r.tier for r in plan.requirements}

    def test_a_documentation_change_requires_no_target_build(self):
        with tempfile.TemporaryDirectory() as root:
            plan = self.plan(root, "docs/note.md")
            self.assertEqual(plan.requirements, ())
            self.assertEqual(plan.undetermined, ())

    def test_a_crate_the_host_gate_skips_requires_the_android_compile(self):
        with tempfile.TemporaryDirectory() as root:
            plan = self.plan(root, "engine/crates/core/src/runtime.rs")
            self.assertEqual(self.platforms(plan), {"android": "compile"})

    def test_a_host_buildable_crates_android_branch_still_requires_it(self):
        # `shared` builds and tests on the host, so nothing about its crate
        # identity asks for a target build. Its `cfg(target_os = "android")`
        # block does.
        with tempfile.TemporaryDirectory() as root:
            plan = self.plan(root, "engine/crates/shared/src/raf_signal.rs")
            self.assertEqual(self.platforms(plan), {"android": "compile"})

    def test_a_windows_branch_in_a_host_crate_requires_the_windows_target(self):
        with tempfile.TemporaryDirectory() as root:
            plan = self.plan(root, "engine/crates/io/src/atomic_write.rs")
            self.assertEqual(self.platforms(plan), {"windows": "compile"})

    def test_a_file_gated_only_by_its_parent_still_names_its_platform(self):
        with tempfile.TemporaryDirectory() as root:
            plan = self.plan(root, "engine/crates/platform/src/android/jni.rs")
            self.assertEqual(self.platforms(plan), {"android": "compile"})

    def test_the_cdylib_requires_a_link_not_only_a_compile(self):
        with tempfile.TemporaryDirectory() as root:
            plan = self.plan(root, "engine/crates/android-jni/src/lib.rs")
            self.assertEqual(self.platforms(plan), {"android": "link"})

    def test_a_link_requirement_absorbs_a_compile_requirement(self):
        with tempfile.TemporaryDirectory() as root:
            plan = self.plan(
                root,
                "engine/crates/core/src/runtime.rs",
                "engine/crates/android-jni/src/lib.rs",
            )
            self.assertEqual(self.platforms(plan), {"android": "link"})

    def test_two_platforms_are_reported_separately(self):
        with tempfile.TemporaryDirectory() as root:
            plan = self.plan(
                root,
                "engine/crates/io/src/atomic_write.rs",
                "engine/crates/platform/src/android/jni.rs",
            )
            self.assertEqual(
                self.platforms(plan), {"windows": "compile", "android": "compile"}
            )

    def test_a_deleted_file_keeps_the_requirement_its_path_carries(self):
        with tempfile.TemporaryDirectory() as root:
            root_path = self.tree(root)
            (root_path / "engine/crates/platform/src/android/jni.rs").unlink()
            plan = targets.select(
                root_path, ["engine/crates/platform/src/android/jni.rs"]
            )
            self.assertEqual(self.platforms(plan), {"android": "compile"})

    def test_a_changed_source_file_with_unknown_conditions_is_not_silently_cleared(self):
        with tempfile.TemporaryDirectory() as root:
            root_path = self.tree(root)
            write(root_path, "engine/crates/shared/src/orphan.rs", "fn f() {}\n")
            plan = targets.select(root_path, ["engine/crates/shared/src/orphan.rs"])
            self.assertEqual(plan.undetermined, ("engine/crates/shared/src/orphan.rs",))

    def test_an_unreached_file_nobody_changed_does_not_block_the_run(self):
        with tempfile.TemporaryDirectory() as root:
            root_path = self.tree(root)
            write(root_path, "engine/crates/shared/src/orphan.rs", "fn f() {}\n")
            plan = targets.select(root_path, ["docs/note.md"])
            self.assertEqual(plan.undetermined, ())

    def test_reasons_name_the_file_and_the_rule_that_selected_it(self):
        with tempfile.TemporaryDirectory() as root:
            plan = self.plan(root, "engine/crates/shared/src/raf_signal.rs")
            (requirement,) = plan.requirements
            self.assertEqual(
                requirement.reasons,
                ("engine/crates/shared/src/raf_signal.rs [cfg]",),
            )

    def test_the_plan_is_ordered_so_two_runs_report_the_same_thing(self):
        with tempfile.TemporaryDirectory() as root:
            changed = [
                "engine/crates/io/src/atomic_write.rs",
                "engine/crates/platform/src/android/jni.rs",
                "engine/crates/core/src/runtime.rs",
            ]
            root_path = self.tree(root)
            first = targets.select(root_path, changed)
            second = targets.select(root_path, list(reversed(changed)))
            self.assertEqual(first, second)

    def test_a_relative_root_resolves_the_same_as_an_absolute_one(self):
        # How the shell entry point calls it: `--root .` from the repository root.
        with tempfile.TemporaryDirectory() as root:
            root_path = self.tree(root)
            absolute = targets.select(root_path, ["engine/crates/io/src/atomic_write.rs"])
            previous = pathlib.Path.cwd()
            try:
                os.chdir(root_path)
                relative = targets.select(
                    pathlib.Path("."), ["engine/crates/io/src/atomic_write.rs"]
                )
            finally:
                os.chdir(previous)
            self.assertEqual(relative, absolute)


class ReportTest(unittest.TestCase):
    """The text the shell entry point parses."""

    def test_a_requirement_is_one_parseable_header_line_plus_its_reasons(self):
        plan = targets.Plan(
            requirements=(
                targets.Requirement("android", "link", ("a.rs [cdylib]", "b.rs [cfg]")),
            ),
            undetermined=(),
            host_packages=(),
        )
        self.assertEqual(
            targets.format_report(plan).splitlines(),
            [
                "TARGET android link",
                "  a.rs [cdylib]",
                "  b.rs [cfg]",
                "HOSTSUITES NONE",
            ],
        )

    def test_undetermined_files_appear_under_their_own_header(self):
        plan = targets.Plan(
            requirements=(), undetermined=("x.rs",), host_packages=()
        )
        self.assertEqual(
            targets.format_report(plan).splitlines(),
            ["HOSTSUITES NONE", "UNDETERMINED", "  x.rs"],
        )

    def test_an_otherwise_empty_plan_still_says_which_host_suites_to_run(self):
        # There is no such thing as a report with no HOSTSUITES line. The shell
        # reads this to decide between running everything and running nothing,
        # and an absent line would be read as neither.
        self.assertEqual(
            targets.format_report(targets.Plan((), ())).splitlines(),
            ["HOSTSUITES ALL"],
        )

    def test_the_three_host_suite_answers_are_each_spelled_out(self):
        # `None` and `()` are different answers that an empty list cannot tell
        # apart, which is the whole reason this line is emitted rather than left
        # implicit. Each is asserted separately so a change that collapses two of
        # them fails here rather than in whichever lane silently stops running.
        self.assertEqual(
            targets.format_report(
                targets.Plan((), (), host_packages=None)
            ).splitlines(),
            ["HOSTSUITES ALL"],
        )
        self.assertEqual(
            targets.format_report(
                targets.Plan((), (), host_packages=())
            ).splitlines(),
            ["HOSTSUITES NONE"],
        )
        self.assertEqual(
            targets.format_report(
                targets.Plan((), (), host_packages=("migo-shared", "migo-io"))
            ).splitlines(),
            ["HOSTSUITES migo-shared migo-io"],
        )

    def test_host_suite_reasons_follow_their_header_and_are_indented(self):
        # Same shape as a requirement's reasons, because the shell parses both by
        # the leading two spaces.
        self.assertEqual(
            targets.format_report(
                targets.Plan(
                    (),
                    (),
                    host_packages=("migo-shared",),
                    host_reasons=("engine/crates/shared/src/lib.rs",),
                )
            ).splitlines(),
            ["HOSTSUITES migo-shared", "  engine/crates/shared/src/lib.rs"],
        )


class AuditTest(unittest.TestCase):
    """Which crate groups the completeness audit walks.

    The audit is the only thing that notices a `mod` form the parser cannot read, so
    a crate group missing from it is a blind spot with no other alarm.
    """

    def test_an_unreached_source_under_engine_crates_is_reported(self):
        with tempfile.TemporaryDirectory() as root:
            write(root, "engine/crates/demo/Cargo.toml", "")
            write(root, "engine/crates/demo/src/lib.rs", "")
            write(root, "engine/crates/demo/src/orphan.rs", "")
            self.assertEqual(
                targets.audit(root), ("engine/crates/demo/src/orphan.rs",)
            )

    def test_an_unreached_source_under_engine_testing_is_reported(self):
        with tempfile.TemporaryDirectory() as root:
            write(root, "engine/testing/probe/Cargo.toml", "")
            write(root, "engine/testing/probe/src/lib.rs", "")
            write(root, "engine/testing/probe/src/orphan.rs", "")
            self.assertEqual(
                targets.audit(root), ("engine/testing/probe/src/orphan.rs",)
            )

    def test_a_group_the_tree_does_not_have_is_not_an_error(self):
        with tempfile.TemporaryDirectory() as root:
            write(root, "engine/crates/demo/Cargo.toml", "")
            write(root, "engine/crates/demo/src/lib.rs", "")
            self.assertEqual(targets.audit(root), ())

    def test_a_directory_without_a_manifest_is_not_a_crate(self):
        with tempfile.TemporaryDirectory() as root:
            write(root, "engine/crates/notacrate/src/orphan.rs", "")
            self.assertEqual(targets.audit(root), ())


if __name__ == "__main__":
    unittest.main()
