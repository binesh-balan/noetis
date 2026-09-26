import { describe, expect, test } from "bun:test";
import { markdownToDocx, markdownToPdf, toBlocks } from "../../src/lib/export-document";

const SAMPLE = `# Meeting Summary: Sync

**Date:** today

## Action Items

- Ship **export**
  - nested *detail*
1. first
2. second

| Owner | Task |
| --- | --- |
| Ana | Review |

---

Plain \`code\` text`;

describe("export-document", () => {
  test("parses summary markdown into blocks", () => {
    const blocks = toBlocks(SAMPLE);
    expect(blocks[0]).toMatchObject({ kind: "heading", level: 1 });
    expect(blocks.find(b => b.kind === "item" && b.depth === 1)).toBeTruthy();
    expect(blocks.filter(b => b.kind === "item" && b.ordered).map(b => (b as any).index)).toEqual([1, 2]);
    const table = blocks.find(b => b.kind === "table") as any;
    expect(table.header.length).toBe(2);
    expect(table.rows[0][0][0].text).toBe("Ana");
    const bold = blocks.flatMap(b => ("runs" in b ? b.runs : [])).find(r => r.bold);
    expect(bold?.text).toBe("Date:");
  });

  test("produces a DOCX (zip) and a PDF", async () => {
    const docx = await markdownToDocx(SAMPLE);
    expect(String.fromCharCode(docx[0], docx[1])).toBe("PK");
    const pdf = await markdownToPdf(SAMPLE);
    expect(new TextDecoder().decode(pdf.slice(0, 5))).toBe("%PDF-");
  });
});
