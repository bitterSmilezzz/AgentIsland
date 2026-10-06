#!/usr/bin/env python3
import ctypes
import contextlib
import io
import importlib.util
import json
import pathlib
import unittest
from unittest.mock import patch

spec=importlib.util.spec_from_file_location('measure',pathlib.Path(__file__).with_name('measure-memory.py'))
measure=importlib.util.module_from_spec(spec);spec.loader.exec_module(measure)

def usage(start=100,footprint=10485760):
    return {'start':start,'resident_size':footprint+100,'phys_footprint':footprint,
            'user_time':12,'system_time':3,'idle_wakeups':1,'interrupt_wakeups':2,'pageins':3,'disk_read':4,'disk_write':5}

class Native:
    def __init__(self):
        self.groups={10:7,20:7,30:8,90:9}
        self.values={10:usage(),20:usage(200)}
        self.calls={}
    def coalition(self,pid):return self.groups.get(pid)
    def processes(self):return [(10,'private-main-path'),(20,'private-user-folder/com.apple.WebKit.WebContent'),(30,'private-unrelated-command')]
    def usage(self,pid):
        self.calls[pid]=self.calls.get(pid,0)+1
        return self.values.get(pid)

class Tests(unittest.TestCase):
    def test_invalid_limits_fail_before_native_access(self):
        for extra in (['--pid','0'],['--pid','10','--seconds','3601'],['--pid','10','--interval','61']):
            with patch.object(measure.sys,'platform','darwin'),patch.object(measure.sys,'argv',['measure',*extra]),patch.object(measure,'Native',side_effect=AssertionError('must not touch native')),contextlib.redirect_stderr(io.StringIO()):
                with self.assertRaises(SystemExit) as result:measure.main()
                self.assertEqual(result.exception.code,2)
    def test_unknown_collector_group_keeps_main_only(self):
        native=Native();native.groups.pop(90)
        result=measure.sample(10,native,collector_pid=90)
        self.assertEqual(result['attribution'],'main-only');self.assertEqual(len(result['members']),1)
    def test_native_layout_matches_installed_sdk(self):
        self.assertEqual(ctypes.sizeof(measure.RusageV2),160)
        self.assertEqual(measure.RusageV2.start.offset,80)
        self.assertEqual(measure.RusageV2.phys_footprint.offset,72)
    def test_missing_footprint_never_becomes_zero_complete_total(self):
        result=measure.summarize([{'footprint_mib':10},{'footprint_mib':None}],'observed-resource-coalition',0,True)
        self.assertEqual(result['known_member_footprint_mib'],10)
        self.assertIsNone(result['total_footprint_mib']);self.assertFalse(result['complete'])
        self.assertIsNone(measure.summarize([], 'main-only',0,True)['total_footprint_mib'])
    def test_same_collector_group_does_not_count_unrelated_app(self):
        native=Native();native.groups[90]=7
        result=measure.sample(10,native,collector_pid=90)
        self.assertEqual(result['attribution'],'main-only');self.assertEqual(len(result['members']),1)
    def test_roles_and_output_are_closed_metadata(self):
        result=measure.sample(10,Native(),collector_pid=90)
        self.assertEqual([item['role'] for item in result['members']],['main','web-content'])
        self.assertEqual(result['total_footprint_mib'],20)
        self.assertNotIn('private-',json.dumps(result));self.assertTrue(result['complete'])
    def test_missing_member_and_unresolved_identity_stay_partial(self):
        native=Native();native.values[20]=None
        result=measure.sample(10,native,collector_pid=90)
        self.assertIsNone(result['total_footprint_mib']);self.assertEqual(result['known_member_footprint_mib'],10)
        native=Native();native.groups.pop(30)
        result=measure.sample(10,native,collector_pid=90)
        self.assertEqual(result['unresolved_membership'],1);self.assertIsNone(result['total_footprint_mib'])
    def test_main_restart_or_reuse_stops_observation(self):
        self.assertEqual(measure.sample(10,Native(),expected_start=99,collector_pid=90)['status'],'target_replaced')
        native=Native();native.values[10]=None
        self.assertEqual(measure.sample(10,native,collector_pid=90)['status'],'target_unavailable')
        native=Native();original=native.usage
        def changing(pid):
            result=original(pid)
            if pid==10 and native.calls[pid]>=4:return usage(101)
            return result
        native.usage=changing
        self.assertEqual(measure.sample(10,native,collector_pid=90)['status'],'target_replaced')
    def test_member_restart_during_sample_cannot_contribute(self):
        native=Native();original=native.usage
        def changing(pid):
            result=original(pid)
            if pid==20 and native.calls[pid]>1:return usage(201)
            return result
        native.usage=changing
        result=measure.sample(10,native,collector_pid=90)
        self.assertFalse(result['membership_stable']);self.assertIsNone(result['total_footprint_mib'])

if __name__=='__main__':unittest.main()
