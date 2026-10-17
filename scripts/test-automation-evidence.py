#!/usr/bin/env python3
"""Evidence cannot turn a selected fragment into complete requirement coverage."""
import json
from pathlib import Path
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts/automation'))
import evidence


class EvidenceTests(unittest.TestCase):
    def setUp(self):
        self.registry = {'requirements': {
            'PARTIAL': {'status': 'partial', 'checks': ['a', 'b']},
            'PLANNED': {'status': 'planned', 'checks': []},
        }}

    def test_a_passing_subset_is_only_partial_execution(self):
        result = evidence.requirement_results(self.registry, [{'check_id': 'a', 'status': 'passed'}])
        self.assertEqual(result['PARTIAL']['execution_status'], 'partial_execution')
        self.assertEqual(result['PARTIAL']['checks']['b'], 'not_selected')
        self.assertFalse(result['PARTIAL']['complete_requirement_verified'])
        self.assertEqual(result['PLANNED']['execution_status'], 'unimplemented')

    def test_all_mapped_checks_still_do_not_prove_complete_product_coverage(self):
        checks = [{'check_id': cid, 'status': 'passed'} for cid in ['a', 'b']]
        result = evidence.requirement_results(self.registry, checks)['PARTIAL']
        self.assertEqual(result['execution_status'], 'mapped_checks_passed')
        self.assertFalse(result['complete_requirement_verified'])

    def test_failure_and_blocked_are_not_erased_by_other_success(self):
        for status in ['failed', 'blocked']:
            checks = [{'check_id': 'a', 'status': 'passed'}, {'check_id': 'b', 'status': status}]
            self.assertEqual(evidence.requirement_results(self.registry, checks)['PARTIAL']['execution_status'], status)

    def test_unselected_is_not_inapplicable(self):
        result = evidence.requirement_results(self.registry, [])['PARTIAL']
        self.assertEqual(result['execution_status'], 'not_selected')
        self.assertNotIn('not_applicable', json.dumps(result))

    def test_input_binding_preserves_snapshot_but_not_raw_private_text(self):
        import hashlib
        source = 'https://github.com/example/repo/issues/12#sha256=' + 'a' * 64
        prompt = 'private fixture text'
        binding = evidence.input_binding('manual-github', source, prompt)
        self.assertEqual(binding['snapshot_sha256'], 'a' * 64)
        self.assertEqual(binding['prompt_sha256'], hashlib.sha256(prompt.encode()).hexdigest())
        self.assertNotIn(prompt, json.dumps(binding))
        self.assertNotIn(source, json.dumps(binding))
        binding = evidence.input_binding('private trigger fixture', 'private source', prompt)
        self.assertEqual(binding['trigger'], 'unknown')
        self.assertNotIn('private', json.dumps(binding))
        self.assertNotIn('snapshot_sha256', binding)

    def test_changed_baselines_invalidate_content_digests_even_with_same_mtime(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'baseline.md'
            path.write_text('first')
            before = evidence.file_digest(path)
            stamp = path.stat().st_mtime_ns
            path.write_text('other')
            import os
            os.utime(path, ns=(stamp, stamp))
            self.assertNotEqual(evidence.file_digest(path), before)
            link = path.with_suffix('.link')
            link.symlink_to(path)
            self.assertIsNone(evidence.file_digest(link))


if __name__ == '__main__':
    unittest.main()
