#!/usr/bin/env python3
"""Repository doctrine checks for the Maestria bootstrap.

Entry point only; the checker lives in the ``philosophy_check`` package
alongside this script, one module per rule family.
"""

from philosophy_check.reporting import main

if __name__ == "__main__":
    raise SystemExit(main())
