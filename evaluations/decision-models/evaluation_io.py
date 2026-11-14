"""Bounded JSONL pipes and cleanup for explicitly owned evaluation workers."""

import json
import os
import selectors
import signal
import time


def stop_owned_group(process):
    """Kill a worker started with start_new_session=True, including descendants."""
    # The leader may have exited while its owned native child is still running.
    process.poll()
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    except PermissionError:
        # Darwin can report EPERM for a group containing only the unreaped leader.
        if process.poll() is None:
            raise
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
    process.wait()


def stop_owned_process(process):
    if process.poll() is None:
        process.kill()
    process.wait()


def remaining(deadline, prefix):
    seconds = deadline-time.monotonic()
    if seconds <= 0:
        raise ValueError(prefix+'_deadline_exceeded')
    return seconds


def jsonl_exchange(process, value, buffer, deadline, *, prefix, max_response_bytes):
    """Exchange one JSON frame before an absolute monotonic deadline.

    Callers own termination after errors. Keep buffer between calls; its byte
    limit includes the newline and any unread tail, matching the actor frames.
    """
    # Buffered writes/readline can block even after select reports readiness.
    remaining(deadline, prefix)
    payload = json.dumps(value, ensure_ascii=False, allow_nan=False).encode()+b'\n'
    remaining(deadline, prefix)
    stdin = process.stdin.fileno()
    stdout = process.stdout.fileno()
    written = 0
    os.set_blocking(stdin, False)
    os.set_blocking(stdout, False)
    with selectors.DefaultSelector() as selector:
        selector.register(stdin, selectors.EVENT_WRITE)
        selector.register(stdout, selectors.EVENT_READ)
        while True:
            remaining(deadline, prefix)
            if len(buffer) > max_response_bytes:
                raise ValueError(prefix+'_output_too_large')
            end = buffer.find(b'\n')
            if end >= 0 and written == len(payload):
                result = json.loads(buffer[:end])
                remaining(deadline, prefix)
                del buffer[:end+1]
                return result
            events = selector.select(remaining(deadline, prefix))
            if not events:
                raise ValueError(prefix+'_deadline_exceeded')
            for key, _ in events:
                remaining(deadline, prefix)
                try:
                    if key.fd == stdin:
                        sent = os.write(stdin, payload[written:written+65536])
                        if not sent:
                            raise ValueError(prefix+'_failed')
                        written += sent
                        if written == len(payload):
                            selector.unregister(stdin)
                    else:
                        part = os.read(stdout, min(65536, max_response_bytes+1-len(buffer)))
                        if not part:
                            raise ValueError(prefix+'_failed')
                        buffer.extend(part)
                except BlockingIOError:
                    continue
