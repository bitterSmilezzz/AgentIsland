#!/usr/bin/env python3
"""Reject missing, shortened, reordered or restarted resource evidence."""
import copy
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('result', Path(__file__).with_name('analyze-memory-soak.py'))
result = importlib.util.module_from_spec(spec)
spec.loader.exec_module(result)


def fixture():
    observation = {'schema_version': 2, 'status': 'observed', 'main_start': 123, 'complete': False,
                   'attribution': 'observed-resource-coalition', 'membership_stable': True,
                   'unresolved_membership': 1, 'total_footprint_mib': None,
                   'members': [{'pid': 10, 'role': 'main', 'footprint_mib': 20}]}
    rows = [{'schema_version': 1, 'phase': 'idle', 'cycle': 0, 'protocol_elapsed': 90 + i * 300,
             'observation': copy.deepcopy(observation)} for i in range(7)]
    for cycle in range(1, 21):
        for phase, offset in [('open', 6), ('released', 96)]:
            rows.append({'schema_version': 1, 'phase': phase, 'cycle': cycle,
                         'protocol_elapsed': 1890 + (cycle - 1) * 96 + offset,
                         'observation': copy.deepcopy(observation)})
    rows.append({'schema_version': 1, 'phase': 'complete', 'protocol_elapsed': rows[-1]['protocol_elapsed'],
                 'protocol_complete': True, 'budget_assessed': False, 'cycles': 20,
                 'idle_seconds': 1800})
    receipt = {'schema': 1, 'exit_code': 0, 'budget_assessed': False, 'started': 1000,
               'finished': 1000 + rows[-1]['protocol_elapsed'] + 1}
    return rows, receipt


class Tests(unittest.TestCase):
    def test_partial_attribution_can_finish_protocol_but_not_budget(self):
        rows, receipt = fixture()
        report = result.analyze(rows, receipt)
        self.assertTrue(report['protocol_complete'])
        self.assertFalse(report['budget_assessed'])
        self.assertEqual(report['partial_samples'], 47)

    def test_complete_attribution_still_cannot_claim_budget(self):
        rows, receipt = fixture()
        for row in rows[:-1]:
            row['observation'].update(complete=True, unresolved_membership=0, total_footprint_mib=20)
        report = result.analyze(rows, receipt)
        self.assertEqual(report['complete_samples'], 47)
        self.assertFalse(report['budget_assessed'])
        rows[0]['observation']['attribution'] = 'main-only'
        self.assertEqual(result.analyze(rows, receipt)['partial_samples'], 1)

    def test_missing_duplicated_and_reordered_cycles_fail(self):
        rows, receipt = fixture()
        for changed in (rows[:-2] + rows[-1:], rows + [rows[-1]],
                        rows[:7] + [rows[8], rows[7]] + rows[9:]):
            with self.subTest(), self.assertRaises(ValueError):
                result.analyze(changed, receipt)

    def test_short_idle_or_wall_clock_fail(self):
        rows, receipt = fixture()
        rows[6]['protocol_elapsed'] -= 1
        with self.assertRaises(ValueError): result.analyze(rows, receipt)
        rows, receipt = fixture()
        rows[-1]['idle_seconds'] = 1799
        with self.assertRaises(ValueError): result.analyze(rows, receipt)
        rows, receipt = fixture()
        receipt['finished'] -= 2
        with self.assertRaises(ValueError): result.analyze(rows, receipt)

    def test_target_replacement_or_missing_main_fail(self):
        for change in ('main_start', 'members'):
            rows, receipt = fixture()
            rows[20]['observation'][change] = 124 if change == 'main_start' else []
            with self.subTest(change=change), self.assertRaises(ValueError):
                result.analyze(rows, receipt)

    def test_failure_and_nonfinite_timestamps_fail(self):
        rows, receipt = fixture()
        receipt['exit_code'] = 2
        with self.assertRaises(ValueError): result.analyze(rows, receipt)
        rows, receipt = fixture()
        rows[-1]['protocol_elapsed'] = float('nan')
        with self.assertRaises(ValueError): result.analyze(rows, receipt)


if __name__ == '__main__': unittest.main()
