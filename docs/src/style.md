# Coding Style Guide

Autopush uses Rust styling guides based on
`cargo fmt` and `cargo clippy`

## Testing Style Guide
Given the integration and load tests are written in Python, we follow a few simple style conventions:
- We conform to the PEP 8 standard [Style Guide][pep8].
- We use type annotations for all variables, functions, and classes.
- We check linting automatically running `make lint` from the root directory. Each subsequent check can be run manually. Consult the [Makefile][makefile] for commands.
- We use [flake8][flake8] as our core style enforcement linter.
- We use [black][black] for formatting and [isort][isort] for import formatting.
- We use [mypy][mypy] for type annotation checking.
- We use [pydocstyle][pydocstyle] for docstring conventions.
- We use [bandit][bandit] for static code security analysis.

## Exceptions

[pep8]: https://pep8.org/
[flake8]: https://flake8.pycqa.org/en/latest/
[black]: https://black.readthedocs.io/en/stable/index.html
[isort]: https://pycqa.github.io/isort/
[pydocstyle]: https://www.pydocstyle.org/en/stable/
[mypy]: https://mypy.readthedocs.io/en/stable/index.html
[bandit]: https://bandit.readthedocs.io/en/latest/
[makefile]: https://github.com/mozilla-services/autopush-rs/blob/master/Makefile