"""Embed the shared library, index a template, and write a PDF."""

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "modules" / "pdf-tooling"))

from pdf_template import PdfTemplateError, Template


def main():
    card = ROOT / "tests" / "fixtures" / "template" / "card.yaml"
    out = ROOT / "target" / "struct-to-pdf" / "template-python.pdf"
    with Template(card.read_bytes(), card.parent) as template:
        fields = template.fields()
        assert fields == ["name", "surname", "company name"], fields
        template.write(
            [
                {"name": "Ann", "surname": "Lee", "company name": "North"},
                {"name": "Bo", "surname": "Kim", "company name": "South"},
            ],
            out,
        )
        data = out.read_bytes()
        assert data.startswith(b"%PDF"), data[:8]
        assert b"/Count 2" in data
        try:
            template.render(
                [
                    {
                        "name": "Ann",
                        "surname": "Lee",
                        "company name": "North",
                        "nope": "x",
                    }
                ]
            )
        except PdfTemplateError as err:
            assert "nope" in str(err), err
        else:
            raise SystemExit("unknown field returned a pdf")
        try:
            template.render([])
        except PdfTemplateError as err:
            assert "no rows" in str(err), err
        else:
            raise SystemExit("zero rows returned a pdf")
    print(out)


if __name__ == "__main__":
    main()
