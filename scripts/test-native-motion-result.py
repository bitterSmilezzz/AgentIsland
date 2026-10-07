#!/usr/bin/env python3
"""Validate native result receipts without starting any native UI."""
import copy
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('motion', Path(__file__).with_name('test-native-motion.py'))
motion = importlib.util.module_from_spec(spec)
spec.loader.exec_module(motion)


class Results(unittest.TestCase):
    def fixture(self):
        frame = dict(t=0, native=dict(x=0, y=0, width=400, height=520, visible=True),
                     card=dict(x=0, y=0, width=300, height=333),
                     viewport=dict(width=400, height=520), navigating=False)
        return dict(schema=1, edge='top', passed=True,
                    traces=[dict(label=label, edge='top', frames=[copy.deepcopy(frame) for _ in range(12)],
                                 assessment=dict(passed=True, failures=[])) for label in motion.LABELS])

    def test_exact_case_coverage_and_edge_required(self):
        result = self.fixture()
        self.assertTrue(motion.valid_result(result, 'top'))
        self.assertFalse(motion.valid_result(result, 'bottom'))
        for change in (lambda r:r.update(passed=False), lambda r:r['traces'].pop(),
                       lambda r:r['traces'].reverse(), lambda r:r['traces'][0].update(edge='left')):
            broken = copy.deepcopy(result); change(broken)
            self.assertFalse(motion.valid_result(broken, 'top'))

    def test_claimed_pass_cannot_replace_real_frames(self):
        for change in (lambda t:t.update(frames=[None]*12), lambda t:t['frames'].pop(),
                       lambda t:t['frames'][-1].update(navigating=True),
                       lambda t:t['frames'][0]['native'].update(visible=False),
                       lambda t:t['frames'][0]['native'].update(width=float('nan')),
                       lambda t:t['assessment'].update(failures=['anchor-drift'])):
            result = self.fixture(); change(result['traces'][0])
            self.assertFalse(motion.valid_result(result, 'top'))


if __name__ == '__main__': unittest.main()
