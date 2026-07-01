# Code Quality Policy

This policy applies to every code, test, documentation, build, and workflow change in this repository.

## No Hacks

The repository prioritizes code quality over immediate results.

If a requested change cannot be completed without introducing a local hack, workaround, monkey patch, partial solution, or fragile old-shape shim, stop and choose one of these paths:

- Fix the underlying flaw in a robust, well-designed, production-ready manner.
- Explain honestly that the request cannot be completed without first adding missing support.

Do not commit code that could predictably break later because it avoided the real design problem. Do not preserve a bad design just to make the current task appear complete.

## Core Values

- Absolute code quality over speed of delivery.
- Correctness over convenience.
- Clarity over cleverness.
- Maintainability over short-term productivity.
- Robust design over quick fixes.
- Simplicity over complexity.
- Doing it right over doing it now.
- Honesty above everything.

## Breaking Changes

Assume this repository is not production software. Correctness, clarity, and maintainability take priority over preserving old behavior.

Do not casually break documented public contracts. If a public API, CLI output shape, persisted format, or documented behavior is flawed, fix it deliberately and update the relevant docs and tests in the same change.

## Reporting

After every change, provide a clear, honest report of any part of the change that is not fully verified or that could still be considered fragile.
