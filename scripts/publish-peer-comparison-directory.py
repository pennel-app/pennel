"""Publish a complete Mac QA directory in one rename, including on rebuilds."""

import ctypes
import errno
import os
import sys


def publish(source, destination):
    try:
        os.rename(source, destination)
    except OSError as error:
        if error.errno not in (errno.EEXIST, errno.ENOTEMPTY):
            raise
        # macOS can exchange two nonempty directories atomically. The old output
        # moves into the caller's private staging directory for normal cleanup.
        rename = ctypes.CDLL(None, use_errno=True).renamex_np
        rename.argtypes = [ctypes.c_char_p, ctypes.c_char_p, ctypes.c_uint]
        rename_swap = 0x00000002  # RENAME_SWAP in <sys/stdio.h>
        if rename(os.fsencode(source), os.fsencode(destination), rename_swap) != 0:
            number = ctypes.get_errno()
            raise OSError(number, os.strerror(number), destination)


if __name__ == "__main__":
    publish(*sys.argv[1:])
