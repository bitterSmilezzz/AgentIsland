#!/usr/bin/env python3
"""Ensure test discovery, variants and profiles remain connected to the runner."""
import json
from pathlib import Path
import re
import shlex
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts/automation'))
import pipeline

BEGIN_MARK = '<!-- generated:requirement-check-mapping:begin -->'
END_MARK = '<!-- generated:requirement-check-mapping:end -->'
HOST_OS = 'windows' if sys.platform == 'win32' else sys.platform


def discover(root):
    scripts = root / 'scripts'
    found = {str(path.relative_to(root)) for pattern in ('test-*.py', 'test-*.mjs', 'test-*.sh') for path in scripts.rglob(pattern) if '__pycache__' not in path.parts and 'node_modules' not in path.parts}
    for directory in ('app/src-tauri/src', 'app/src-tauri/tests'):
        rust = root / directory
        found.update(str(path.relative_to(root)) for path in rust.rglob('*.rs')
                     if re.search(r'#\[(?:test|(?:tokio::)?test\([^\]]*\))\]', path.read_text(encoding='utf-8')))
    return found


def audit_ci_prerequisites(registry, workflow):
    """Check the repository's explicit job/step/run layout, not arbitrary YAML/shell.

    Only unconditional, exact installation commands establish prerequisites. Unknown
    syntax cannot establish success; Actions remains the runtime authority.
    """
    needs_npm = any('npm' in (registry['checks'][cid].get('external_requirement') or '')
                    for cid in registry['profiles'].get('ci', []))
    if not needs_npm:
        return []
    if workflow is None:
        return ['CI prerequisite: workflow is missing']
    jobs = []
    job = None
    step = None
    body_indent = None
    for line in workflow.splitlines():
        if not line.strip() or line.lstrip().startswith('#'):
            continue
        indent = len(line) - len(line.lstrip(' '))
        if body_indent is not None and indent > body_indent:
            step['run'].append(line.strip())
            continue
        body_indent = None
        if re.fullmatch(r'  [\w-]+:', line):
            job = []
            jobs.append(job)
            step = None
            continue
        if job is None:
            continue
        match = re.match(r'^      - (.*)\Z', line)
        if match:
            step = {'run': [], 'conditional': False}
            job.append(step)
            value = match[1]
        elif step is not None and indent == 8:
            value = line.strip()
        else:
            continue
        if value.startswith(('if:', 'continue-on-error:')):
            step['conditional'] = True
        if value.startswith('run:'):
            command = value[4:].strip()
            if command in ('|', '|-', '|+', '>', '>-', '>+'):
                body_indent = 8
            else:
                step['run'].append(command)
    errors = []
    consumers = 0
    allowed_installs = {
        ('npm', 'ci'): 'npm ci',
        ('npx', '--no-install', 'playwright', 'install', 'chromium'): 'playwright chromium',
    }
    for job in jobs:
        installed = set()
        for step in job:
            parsed = []
            for command in step['run']:
                try:
                    argv = shlex.split(command, comments=True)
                except ValueError:
                    argv = []
                if ('scripts/automation/pipeline.py' in argv and '--profile' in argv
                        and argv[argv.index('--profile') + 1:][:1] == ['ci']):
                    consumers += 1
                    missing = {'npm ci', 'playwright chromium'} - installed
                    if missing:
                        errors.append('CI prerequisite: missing earlier unconditional install: '
                                      + ', '.join(sorted(missing)))
                parsed.append(tuple(argv))
            # Accept dedicated install steps only. A matching line inside a heredoc,
            # `if false`, function, or other shell control flow is not execution proof.
            if (not step['conditional'] and parsed
                    and all(command in allowed_installs for command in parsed)):
                installed.update(allowed_installs[command] for command in parsed)
    if not consumers:
        errors.append('CI prerequisite: no supported job runs the ci profile')
    return errors


def audit(registry, discovered, root=ROOT):
    errors = []
    declared = {p for entry in registry['checks'].values() for p in entry.get('test_files', [])}
    for path in sorted(discovered - declared):
        errors.append(f'unregistered test: {path}')
    for path in sorted(declared):
        if not (root / path).is_file():
            errors.append(f'missing test: {path}')
    # One test file must map to exactly one check id, unless the extra ids are declared
    # shell variants (same script, different command_env). An undeclared second id lets
    # two gates drift apart silently: one gets fixed, the other keeps stale assertions
    # and still reports green.
    owners: dict[str, list[str]] = {}
    for cid, entry in registry['checks'].items():
        for path in entry.get('test_files', []):
            owners.setdefault(path, []).append(cid)
    for path, cids in sorted(owners.items()):
        if len(cids) < 2:
            continue
        variants = {cid for cid in cids if registry['checks'][cid].get('command_env')}
        base = set(cids) - variants
        if len(base) > 1:
            errors.append(f'duplicate check ids for {path}: {", ".join(sorted(base))}')
        for cid in sorted(variants):
            if not registry['checks'][cid].get('command_env'):
                errors.append(f'variant check without command_env: {cid}')
    # A coverage note that names a source file must still exist: stale notes claimed
    # deleted symbols as evidence, which is worse than saying nothing.
    for rid, entry in registry['requirements'].items():
        note = entry.get('coverage_note', '')
        for name in re.findall(r'[\w./-]+\.rs', note):
            if not (root / 'app/src-tauri/src' / name).is_file() and not (root / name).is_file():
                errors.append(f'{rid} coverage note references missing source: {name}')
    # `maps_to_requirement` was a second copy of the check->requirement mapping that
    # nothing ever read: `evidence.py` builds coverage from `requirements[].checks`
    # alone. One check can serve several requirements, so the copy silently kept only
    # the first one and drifted (task01.detailnav and ops01.measuremem both claimed a
    # single requirement while actually serving two). Deleted rather than kept in sync;
    # this keeps it from coming back as a stale duplicate.
    for cid, entry in registry['checks'].items():
        if 'maps_to_requirement' in entry:
            errors.append(
                f'{cid} still carries maps_to_requirement: the authoritative mapping is '
                'requirements[].checks, and a second copy only drifts')
    # `audit_ref` points a reader at the audit document behind a requirement. Nothing
    # executes it, so a renamed or deleted document leaves a reference that still looks
    # valid — worse than no reference, because it is trusted without being checked.
    for rid, entry in registry['requirements'].items():
        ref = entry.get('audit_ref')
        if not ref:
            continue
        target = re.match(r'[\w./-]+\.(?:md|py|rs|sh|json)', ref)
        if not target or not (root / target.group(0)).is_file():
            errors.append(f'{rid} audit_ref references missing document: {ref}')
    profiles = registry['profiles']
    selected = {cid for ids in profiles.values() if isinstance(ids, list) for cid in ids}
    for cid in registry['checks']:
        if cid not in selected:
            errors.append(f'check has no profile: {cid}')
    # ⚠️ 这里**故意不检查「文件是否已入库」**。
    #
    # 那条判据做过、被撤回过一次（见 docs/测试自动化交付.md 的 `automation.tracked`）：
    # `git ls-files` 只证明索引登记——暂存未提交也会通过，**证明不了远端 CI 能跑通**；
    # 而它会把正常的未提交开发状态判成代码失败，让每次本地反馈都红，直到提交范围敲定。
    # 「干净检出能否跑通」是分发完整性问题，只能由远端 Actions 的真实结果回答。
    #
    workflow = root / '.github/workflows/build.yml'
    errors.extend(audit_ci_prerequisites(
        registry, workflow.read_text(encoding='utf-8') if workflow.is_file() else None))
    baseline = root / 'docs/需求.md'
    if baseline.is_file():
        requirements = set(re.findall(r'^\| ((?:SUP|GW|TASK|NOT|AUTO|SEC|OPS)-\d+) \|', baseline.read_text(), re.M))
        if set(registry['requirements']) != requirements:
            errors.append('requirement IDs differ from the requirement baseline')
    required_variants = {
        'shell.workbench': {'TEST_NAVIGATION_SHELL': 'workbench'},
        'sup05.deeplink.workbench': {'TEST_NAVIGATION_SHELL': 'workbench'},
        'sup05.deeplink.cold': {'TEST_COLD_NAVIGATION': '1'},
    }
    for cid, required in required_variants.items():
        if cid not in profiles['ci'] or registry['checks'].get(cid, {}).get('command_env') != required:
            errors.append('required shell variant missing')
    if 'rust.manual' in profiles['ci'] or 'rust.manual' in profiles['native']:
        errors.append('private probes cannot run in automatic profiles')
    # An opt-in check must not be reachable from a profile that runs unattended.
    for cid, entry in registry['checks'].items():
        required = entry.get('requires_env')
        if isinstance(required, dict) and cid in profiles['ci']:
            errors.append(f'opt-in check must stay out of ci: {cid}')
    # Browser gates must go through the pinned-runtime wrapper, never the raw script.
    for cid in profiles['browser']:
        if registry['checks'][cid]['command'][1] != 'scripts/automation/browser-check.mjs':
            errors.append(f'browser check bypasses the pinned runtime: {cid}')
    # A check that runs against a built bundle must prove the bundle is the current
    # build. The mtime baseline was replaced by a content-manifest receipt check; the
    # rule now demands that stronger evidence instead of accepting a timestamps guess.
    verified = [cid for cid in profiles['package'] if cid == 'package.artifact']
    if not verified:
        errors.append('package profile must verify build identity before running bundle checks')
    for cid in profiles['package']:
        entry = registry['checks'][cid]
        if entry.get('requires_binary') and 'binary_newer_than' in entry:
            errors.append(f'package check still relies on mtime freshness: {cid}')
        if entry.get('requires_binary') and 'package.artifact' not in entry.get('depends_on', []):
            errors.append(f'package check must depend on build identity: {cid}')
    if any(registry['checks'][cid]['command'][1] != 'scripts/native-gates-vm.py' for cid in profiles['native']):
        errors.append('native must use the verified VM orchestrator')
    # The matrix's generated mapping must match the registry, or the document claims a
    # coverage picture the gate does not implement.
    matrix = root / 'docs/需求测试矩阵.md'
    if matrix.is_file():
        text = matrix.read_text(encoding='utf-8')
        if BEGIN_MARK not in text or END_MARK not in text:
            errors.append('requirement matrix is missing its generated mapping block')
        else:
            block = text[text.index(BEGIN_MARK):text.index(END_MARK)]
            for cid, entry in registry['checks'].items():
                if f'`{cid}`' not in block:
                    errors.append(f'registry check absent from the matrix mapping: {cid}')
            for rid, entry in registry['requirements'].items():
                if not entry.get('checks') and f'| {rid} | planned' not in block:
                    errors.append(f'planned requirement absent from the matrix mapping: {rid}')
    return errors


class RegistryTests(unittest.TestCase):
    def setUp(self):
        self.registry = pipeline.load_registry()
        self.discovered = discover(ROOT)

    def test_no_profile_is_an_unreferenced_alias(self):
        # A profile that duplicates another profile's exact contents advertises
        # coverage no run ever produces; recovery was exactly that before it was
        # removed. A distinct profile is only honest if it really differs.
        profiles = self.registry['profiles']
        seen: dict[tuple, str] = {}
        for name, ids in profiles.items():
            if not isinstance(ids, list):
                continue
            with self.subTest(profile=name):
                key = tuple(ids)
                self.assertNotIn(key, seen,
                                 f"{name} duplicates {seen.get(key)} exactly")
                seen[key] = name
        # Profiles that no script or document names cannot be run by anyone.
        roots = [ROOT / 'scripts', ROOT / 'docs', ROOT / '.github']
        text = "\n".join(path.read_text(encoding='utf-8', errors='ignore')
                         for root in roots if root.is_dir()
                         for path in root.rglob('*') if path.is_file()
                         and path.suffix in ('.sh', '.py', '.yml', '.md'))
        for name in profiles:
            if not isinstance(profiles[name], list):
                continue
            with self.subTest(caller=name):
                self.assertIn(name, text, f"no caller mentions profile: {name}")

    def test_a_long_protocol_declares_its_own_timeout_budget(self):
        # A check whose protocol legitimately outruns the profile default must
        # declare a longer budget, otherwise it can only ever report "timeout".
        for cid, entry in self.registry['checks'].items():
            with self.subTest(check=cid):
                if cid == 'soak.resource':
                    self.assertGreaterEqual(entry['timeout_seconds'], 6300,
                                            "the soak lease itself is 6300s")
                if 'timeout_seconds' in entry:
                    self.assertIs(type(entry['timeout_seconds']), int)
                    self.assertTrue(60 <= entry['timeout_seconds'] <= 86400)
        for mutate in (lambda e: e.update(timeout_seconds=0),
                       lambda e: e.update(timeout_seconds='soon'),
                       lambda e: e.update(timeout_seconds=99999)):
            registry = json.loads(json.dumps(self.registry))
            mutate(registry['checks']['soak.resource'])
            with self.assertRaises(pipeline.PipelineError):
                pipeline.validate_registry(registry)

    def test_every_check_is_reached_by_the_full_verifier(self):
        # A check no invoked profile carries is coverage that never happens.
        # Derive the invoked set from the script itself (comments stripped, since
        # a comment naming a profile is not a run), then require that the union of
        # those profiles covers every registered check. Stating a subset rule here
        # instead would let a profile be skipped as "covered" by itself.
        verifier = (ROOT / "scripts/verify-framework.sh").read_text(encoding="utf-8")
        code = "\n".join(line for line in verifier.splitlines()
                         if not line.lstrip().startswith("#"))
        invoked = set(re.findall(r"run_profile\s+\"?([A-Za-z_][\w-]*)\"?", code))
        # Only the external-prerequisite loop lists profiles; the meta-suite loop
        # lists test scripts and must not be read as profile names.
        for group in re.findall(r"for p in ([^;]+?);\s*do", code):
            invoked.update(group.split())
        self.assertTrue(invoked, "verifier invokes no profile at all")
        reached = set()
        for name in invoked:
            self.assertIn(name, self.registry['profiles'], f"unknown profile invoked: {name}")
            reached |= set(self.registry['profiles'][name])
        missing = sorted(set(self.registry['checks']) - reached)
        self.assertEqual(missing, [], f"the unified verifier never reaches: {missing}")

    def test_declared_counterexamples_are_well_formed(self):
        # A check may claim "proven able to fail" only with a real, runnable
        # counterexample; a malformed or runner-invoking one is not evidence.
        declared = [cid for cid, entry in self.registry['checks'].items()
                    if 'discriminating_by' in entry]
        self.assertTrue(declared, "no check declares a counterexample")
        for cid in declared:
            with self.subTest(check=cid):
                counter = self.registry['checks'][cid]['discriminating_by']
                self.assertTrue(counter['reason'].strip())
                script = ROOT / counter['command'][1]
                self.assertTrue(script.is_file(), f"missing counterexample script: {script}")
                if '--ablation-experiments' in counter['command']:
                    self.assertIn('ablation_experiments', script.read_text(encoding='utf-8'))

    def test_declared_counterexamples_actually_prove_the_check_can_fail(self):
        # Grepping the script for a keyword proves the experiment exists, not that it
        # discriminates. This local regression supplements the isolated break-* gates;
        # those remain the authority for strict protocol and process containment.
        declared = [cid for cid, entry in self.registry['checks'].items()
                    if 'discriminating_by' in entry]
        self.assertTrue(declared, "no check declares a counterexample")
        for cid in sorted(declared):
            with self.subTest(check=cid):
                command = list(self.registry['checks'][cid]['discriminating_by']['command'])
                command[0] = sys.executable
                result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, timeout=600)
                self.assertEqual(result.returncode, 0,
                                 f'{cid} 反例命令执行失败: {result.stderr[-400:]}')
                payload = json.loads(result.stdout)
                experiments = payload.get('experiments') or []
                self.assertTrue(experiments, f'{cid} 反例没有产出任何实验')
                for experiment in experiments:
                    self.assertTrue(
                        experiment.get('mutant_false_green'),
                        f'{cid} 的消融实验没有证明「去掉该能力就会假绿」: {experiment}')

    def test_a_malformed_counterexample_is_rejected(self):
        for mutate in (lambda c: c.update(command=[]),
                       lambda c: c.update(reason=''),
                       lambda c: c.update(extra='x'),
                       lambda c: c.pop('reason')):
            registry = json.loads(json.dumps(self.registry))
            counter = registry['checks']['automation.artifact']['discriminating_by']
            mutate(counter)
            with self.assertRaises(pipeline.PipelineError):
                pipeline.validate_registry(registry)

    def test_all_assets_requirements_and_profiles_are_connected(self):
        self.assertEqual(audit(self.registry, self.discovered), [])

    def test_a_matrix_mapping_that_drops_a_check_is_rejected(self):
        registry = json.loads(json.dumps(self.registry))
        registry['checks']['orphan.check'] = {'command': ['python3', '-c', 'pass'],
                                              'host_os': [HOST_OS]}
        registry['profiles']['ci'].append('orphan.check')
        errors = audit(registry, self.discovered)
        self.assertIn('registry check absent from the matrix mapping: orphan.check', errors)

    def test_a_matrix_mapping_that_hides_a_planned_requirement_is_rejected(self):
        registry = json.loads(json.dumps(self.registry))
        registry['requirements']['PLANNED-99'] = {'checks': [], 'status': 'planned'}
        errors = audit(registry, self.discovered)
        self.assertIn('planned requirement absent from the matrix mapping: PLANNED-99', errors)

    def test_real_new_files_in_all_scanned_layers_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name, text in [('scripts/test-fixture.py', ''), ('scripts/nested/test-fixture.mjs', ''),
                               ('app/src-tauri/src/new_module.rs', '#[test]\nfn fixture() {}'),
                               ('app/src-tauri/tests/integration.rs', '#[test]\nfn integration() {}')]:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(text)
            discovered = discover(root)
            self.assertEqual(len(discovered), 4)
            errors = audit(self.registry, discovered, root)
            for name in discovered:
                self.assertIn(f'unregistered test: {name}', errors)

    def test_a_deleted_registered_file_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            errors = audit(self.registry, set(), Path(directory))
            self.assertIn('missing test: scripts/test-tasks.mjs', errors)

    def test_one_test_file_cannot_back_two_check_ids(self):
        registry = json.loads(json.dumps(self.registry))
        registry['checks']['duplicate.alias'] = {
            'command': ['python3', 'scripts/test-tasks.mjs'],
            'host_os': ['darwin'], 'test_files': ['scripts/test-tasks.mjs'],
        }
        registry['profiles']['ci'].append('duplicate.alias')
        errors = audit(registry, self.discovered)
        self.assertIn('duplicate check ids for scripts/test-tasks.mjs: duplicate.alias, task01.tasks',
                      errors)

    def test_a_coverage_note_cannot_cite_a_missing_source_file(self):
        registry = json.loads(json.dumps(self.registry))
        registry['requirements']['SUP-02']['coverage_note'] = 'partial — see deleted_module.rs'
        errors = audit(registry, self.discovered)
        self.assertIn('SUP-02 coverage note references missing source: deleted_module.rs', errors)

    def test_a_deleted_shell_variant_is_rejected(self):
        registry = json.loads(json.dumps(self.registry))
        for entry in registry['checks'].values():
            entry.pop('command_env', None)
        self.assertIn('required shell variant missing', audit(registry, self.discovered))

    def test_each_shell_variant_is_required_individually(self):
        for cid in ('shell.workbench', 'sup05.deeplink.workbench', 'sup05.deeplink.cold'):
            with self.subTest(cid=cid):
                registry = json.loads(json.dumps(self.registry))
                registry['profiles']['ci'].remove(cid)
                self.assertIn('required shell variant missing', audit(registry, self.discovered))

    def test_release_includes_every_ci_check_and_package_identity(self):
        profiles = self.registry['profiles']
        self.assertEqual(set(profiles['release']), set(profiles['ci']) | set(profiles['package']))

    def test_runtime_and_artifact_guards_cannot_be_bypassed(self):
        registry = json.loads(json.dumps(self.registry))
        registry['checks']['task01.detailnav']['command'][1] = 'scripts/test-task-detail-navigation.mjs'
        self.assertIn('browser check bypasses the pinned runtime: task01.detailnav', audit(registry, self.discovered))
        # Removing the build-identity check must be rejected: the remaining bundle
        # checks would then run against whatever stale artifact happens to exist.
        registry = json.loads(json.dumps(self.registry))
        registry['profiles']['package'].remove('package.artifact')
        self.assertIn('package profile must verify build identity before running bundle checks',
                      audit(registry, self.discovered))
        registry = json.loads(json.dumps(self.registry))
        registry['checks']['sup01.clidispatch'].pop('depends_on')
        self.assertIn('package check must depend on build identity: sup01.clidispatch',
                      audit(registry, self.discovered))
        # Falling back to an mtime guess must also be rejected.
        registry = json.loads(json.dumps(self.registry))
        registry['checks']['sup01.clidispatch']['binary_newer_than'] = ['app/src-tauri/src/main.rs']
        self.assertIn('package check still relies on mtime freshness: sup01.clidispatch',
                      audit(registry, self.discovered))

    def test_manual_probes_cannot_silently_enter_ci(self):
        registry = json.loads(json.dumps(self.registry))
        registry['profiles']['ci'].append('rust.manual')
        self.assertIn('private probes cannot run in automatic profiles', audit(registry, self.discovered))

    def test_the_dead_requirement_mapping_cannot_come_back(self):
        # `maps_to_requirement` duplicated `requirements[].checks` and nothing read it.
        # A check can serve several requirements, so the copy kept only the first and
        # drifted. Deleting it is not enough — prove the ban actually fires.
        registry = json.loads(json.dumps(self.registry))
        registry['checks']['rust.default']['maps_to_requirement'] = ['SEC-01']
        errors = audit(registry, self.discovered)
        self.assertTrue(
            any('maps_to_requirement' in error and 'rust.default' in error for error in errors),
            f'规则没抓住死字段复活，实际错误列表: {errors}')

    def test_an_audit_ref_cannot_point_at_a_document_that_is_gone(self):
        # Nothing executes audit_ref, so a deleted document leaves a reference that
        # still reads as valid. That is worse than no reference: it is trusted
        # without ever being checked.
        registry = json.loads(json.dumps(self.registry))
        with_ref = [rid for rid, entry in registry['requirements'].items() if entry.get('audit_ref')]
        self.assertTrue(with_ref, '没有任何 requirement 带 audit_ref，这条守护没有可保护的对象')
        registry['requirements'][with_ref[0]]['audit_ref'] = 'docs/已被删除的盘点.md'
        errors = audit(registry, self.discovered)
        self.assertTrue(
            any('audit_ref references missing document' in error for error in errors),
            f'规则没抓住死链接，实际错误列表: {errors}')

    def test_ci_prerequisites_must_be_installed_by_the_workflow_that_runs_ci(self):
        # ci declares an npm prerequisite; the workflow must install it. Otherwise the
        # registry keeps claiming its own dependency is met while the browser checks
        # fail at runtime — somewhere else, later.
        registry = json.loads(json.dumps(self.registry))
        needs_npm = [cid for cid in registry['profiles']['ci']
                     if 'npm' in (registry['checks'][cid].get('external_requirement') or '')]
        self.assertTrue(needs_npm, 'ci 里没有声明 npm 前提的检查，这条守护没有可保护的对象')
        original = (ROOT / '.github/workflows/build.yml').read_text(encoding='utf-8')
        install = '          npm ci\n'
        browser = '          npx --no-install playwright install chromium\n'
        variants = {
            'removed': original.replace(install, '', 1),
            'comment_only': original.replace(install, '          # npm ci\n', 1),
            'dry_run': original.replace(install, '          npm ci --dry-run\n', 1),
            'wrong_job': original.replace(install, '', 1) + '\n      - run: npm ci\n',
            'after_ci': original.replace(install, '', 1).replace(
                '      - name: 安装固定版本的 Tauri 打包工具',
                '      - run: npm ci\n      - name: 安装固定版本的 Tauri 打包工具', 1),
            'missing_browser': original.replace(browser, '', 1),
            'shell_condition': original.replace(
                install, '          if false; then\n' + install + '          fi\n', 1),
            'heredoc': original.replace(
                install, "          cat <<'UNUSED'\n" + install + '          UNUSED\n', 1),
            'conditional_install': original.replace(
                '      - name: 安装锁定的浏览器运行时',
                '      - if: ${{ false }}\n        name: 安装锁定的浏览器运行时', 1),
        }
        # Exercise the real audit on disposable files. The source workflow must never
        # be a mutation fixture: parallel source.integrity checks observe those writes.
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            workflow = root / '.github/workflows/build.yml'
            workflow.parent.mkdir(parents=True)
            workflow.write_text(original, encoding='utf-8')
            self.assertFalse([e for e in audit(registry, set(), root)
                              if 'CI prerequisite' in e])
            for name, text in variants.items():
                with self.subTest(variant=name):
                    self.assertNotEqual(text, original, 'mutation did not apply')
                    workflow.write_text(text, encoding='utf-8')
                    errors = audit(registry, set(), root)
                    self.assertTrue(any('CI prerequisite' in e for e in errors),
                                    [e for e in errors if 'CI prerequisite' in e])
            workflow.unlink()
            self.assertIn('CI prerequisite: workflow is missing', audit(registry, set(), root))
        self.assertEqual((ROOT / '.github/workflows/build.yml').read_text(encoding='utf-8'),
                         original)


if __name__ == '__main__':
    unittest.main()
