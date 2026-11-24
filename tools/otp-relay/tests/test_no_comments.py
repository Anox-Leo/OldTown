import ast
import io
import tokenize
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def comments(src: str) -> list[int]:
    found = [
        t.start[0]
        for t in tokenize.generate_tokens(io.StringIO(src).readline)
        if t.type == tokenize.COMMENT and not (t.start[0] == 1 and t.string.startswith("#!"))
    ]
    for node in ast.walk(ast.parse(src)):
        body = getattr(node, "body", None)
        for statement in body if isinstance(body, list) else []:
            if (
                isinstance(statement, ast.Expr)
                and isinstance(statement.value, ast.Constant)
                and isinstance(statement.value.value, str)
            ):
                found.append(statement.lineno)
    return sorted(found)


class NoCommentsTest(unittest.TestCase):
    def test_lexer(self) -> None:
        src = '#!/usr/bin/env python3\n"""oui"""\nx = "# pas"\n\n\ndef f():\n    """oui"""\n    return x  # oui\n'
        self.assertEqual(comments(src), [2, 7, 8])

    def test_no_comments(self) -> None:
        files = sorted(p for p in ROOT.rglob("*.py") if "__pycache__" not in p.parts)
        self.assertGreater(len(files), 10)
        offenders = [
            f"{path.relative_to(ROOT)}:{line}"
            for path in files
            for line in comments(path.read_text(encoding="utf-8"))
        ]
        self.assertEqual(offenders, [])


if __name__ == "__main__":
    unittest.main()
