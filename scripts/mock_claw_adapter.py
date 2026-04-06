#!/usr/bin/env python3
import sys


def main() -> int:
    print("mock-claw: ready", flush=True)
    print("mock-claw: stderr online", file=sys.stderr, flush=True)

    buffer = []
    for raw_line in sys.stdin:
        line = raw_line.rstrip("\r\n")
        if not line:
            if buffer:
                print("mock-claw: received task envelope", flush=True)
                for item in buffer:
                    print(f"mock-claw: {item}", flush=True)
                print("PROXY_CMD git status", flush=True)
                buffer.clear()
            continue

        buffer.append(line)

    if buffer:
        print("mock-claw: received trailing envelope", flush=True)
        for item in buffer:
            print(f"mock-claw: {item}", flush=True)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
