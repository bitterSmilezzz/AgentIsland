#!/usr/bin/env python3
"""Exec a command holding a POSIX advisory lock, inherited by its descendants.

The file stays in place: deleting it would let another caller lock a different inode.
Kernel ownership avoids PID reuse and stale-directory recovery races. Receipt state
remains independent and is never cleared by this helper.
"""
import errno
import fcntl
import json
import os
from pathlib import Path
import stat
import sys


def main():
    args = sys.argv[1:]
    if len(args) < 4 or args[0] != '--root' or args[2] != '--':
        print('usage: heavy-build-lock.sh --root PATH -- COMMAND [ARG ...]', file=sys.stderr)
        return 2
    try:
        root = Path(args[1]).resolve(strict=True)
        directory = root / '.build'
        directory.mkdir(mode=0o700, exist_ok=True)
        if directory.is_symlink() or not directory.is_dir():
            raise OSError('unsafe lock directory')
        fd = os.open(directory / 'heavy-build.lock',
                     os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW | os.O_NONBLOCK, 0o600)
        info = os.fstat(fd)
        if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1 or info.st_uid != os.getuid():
            os.close(fd)
            raise OSError('unsafe lock file')
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError as exc:
            os.close(fd)
            if exc.errno in (errno.EAGAIN, errno.EACCES):
                print(json.dumps({'ok': False, 'code': 'heavy_build_busy'}), file=sys.stderr)
                return 2
            raise
        os.fchmod(fd, 0o600)
        # exec keeps the PID and signal/exit behavior of the real command. Descendants
        # inherit the descriptor, so killing a parent cannot unlock a still-running child.
        os.set_inheritable(fd, True)
        os.execvp(args[3], args[3:])
    except OSError:
        print(json.dumps({'ok': False, 'code': 'heavy_build_lock_error'}), file=sys.stderr)
        return 2


if __name__ == '__main__':
    raise SystemExit(main())
