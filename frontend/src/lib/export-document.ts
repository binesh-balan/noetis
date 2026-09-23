// Markdown -> DOCX / PDF for meeting exports. Both libraries are loaded on demand so they
// stay out of the main bundle. Covers what summaries actually contain: headings,
// paragraphs, (nested) lists, tables, rules, bold/italic/code.
import { marked, type Token, type Tokens } from 'marked';

export type ExportFormat = 'pdf' | 'docx' | 'md';

interface Run { text: string; bold?: boolean; italic?: boolean; code?: boolean }
type Block =
  | { kind: 'heading'; level: number; runs: Run[] }
  | { kind: 'paragraph'; runs: Run[] }
  | { kind: 'item'; ordered: boolean; index: number; depth: number; runs: Run[] }
  | { kind: 'table'; header: Run[][]; rows: Run[][][] }
  | { kind: 'rule' };

function inline(tokens: Token[] | undefined, style: Omit<Run, 'text'> = {}): Run[] {
  if (!tokens) return [];
  return tokens.flatMap((t): Run[] => {
    switch (t.type) {
      case 'strong': return inline((t as Tokens.Strong).tokens, { ...style, bold: true });
      case 'em': return inline((t as Tokens.Em).tokens, { ...style, italic: true });
      case 'del': return inline((t as Tokens.Del).tokens, style);
      case 'link': return inline((t as Tokens.Link).tokens, style);
      case 'codespan': return [{ ...style, code: true, text: (t as Tokens.Codespan).text }];
      case 'br': return [{ ...style, text: '\n' }];
      case 'text': {
        const tt = t as Tokens.Text;
        return tt.tokens ? inline(tt.tokens, style) : [{ ...style, text: tt.text }];
      }
      default: return 'text' in t ? [{ ...style, text: String(t.text) }] : [];
    }
  });
}

export function toBlocks(markdown: string): Block[] {
  const out: Block[] = [];
  const walk = (tokens: Token[], depth: number) => {
    for (const t of tokens) {
      switch (t.type) {
        case 'heading': out.push({ kind: 'heading', level: t.depth, runs: inline(t.tokens) }); break;
        case 'paragraph': out.push({ kind: 'paragraph', runs: inline(t.tokens) }); break;
        case 'hr': out.push({ kind: 'rule' }); break;
        case 'code': out.push({ kind: 'paragraph', runs: [{ text: t.text, code: true }] }); break;
        case 'blockquote': walk(t.tokens ?? [], depth); break;
        case 'table': {
          const tb = t as Tokens.Table;
          out.push({ kind: 'table', header: tb.header.map(c => inline(c.tokens)), rows: tb.rows.map(r => r.map(c => inline(c.tokens))) });
          break;
        }
        case 'list': {
          const list = t as Tokens.List;
          list.items.forEach((item, i) => {
            // An item's own text is its leading text/paragraph tokens; nested lists recurse.
            const own = item.tokens.filter(x => x.type !== 'list');
            const runs = own.flatMap(x => ('tokens' in x && x.tokens ? inline(x.tokens as Token[]) : 'text' in x ? [{ text: String(x.text) }] : []));
            out.push({ kind: 'item', ordered: list.ordered, index: (Number(list.start) || 1) + i, depth, runs });
            walk(item.tokens.filter(x => x.type === 'list'), depth + 1);
          });
          break;
        }
      }
    }
  };
  walk(marked.lexer(markdown), 0);
  return out;
}

export async function markdownToDocx(markdown: string): Promise<Uint8Array> {
  const d = await import('docx');
  const runs = (rs: Run[]) => rs.map(r => new d.TextRun({ text: r.text, bold: r.bold, italics: r.italic, font: r.code ? 'Consolas' : undefined, break: r.text === '\n' ? 1 : undefined }));
  const HEADINGS = [d.HeadingLevel.HEADING_1, d.HeadingLevel.HEADING_2, d.HeadingLevel.HEADING_3, d.HeadingLevel.HEADING_4, d.HeadingLevel.HEADING_5, d.HeadingLevel.HEADING_6];

  const children = toBlocks(markdown).map(b => {
    switch (b.kind) {
      case 'heading': return new d.Paragraph({ heading: HEADINGS[b.level - 1], children: runs(b.runs) });
      case 'paragraph': return new d.Paragraph({ children: runs(b.runs) });
      case 'rule': return new d.Paragraph({ border: { bottom: { style: d.BorderStyle.SINGLE, size: 6, color: '999999', space: 1 } } });
      case 'item': return b.ordered
        ? new d.Paragraph({ numbering: { reference: 'ordered', level: Math.min(b.depth, 8) }, children: runs(b.runs) })
        : new d.Paragraph({ bullet: { level: Math.min(b.depth, 8) }, children: runs(b.runs) });
      case 'table': {
        const row = (cells: Run[][], header: boolean) => new d.TableRow({
          tableHeader: header,
          children: cells.map(c => new d.TableCell({ children: [new d.Paragraph({ children: runs(header ? c.map(r => ({ ...r, bold: true })) : c) })] })),
        });
        return new d.Table({ width: { size: 100, type: d.WidthType.PERCENTAGE }, rows: [row(b.header, true), ...b.rows.map(r => row(r, false))] });
      }
    }
  });

  const doc = new d.Document({
    numbering: {
      config: [{
        reference: 'ordered',
        levels: Array.from({ length: 9 }, (_, level) => ({
          level, format: d.LevelFormat.DECIMAL, text: `%${level + 1}.`, alignment: d.AlignmentType.START,
          style: { paragraph: { indent: { left: 720 * (level + 1), hanging: 360 } } },
        })),
      }],
    },
    sections: [{ children }],
  });
  return new Uint8Array(await (await d.Packer.toBlob(doc)).arrayBuffer());
}

export async function markdownToPdf(markdown: string): Promise<Uint8Array> {
  const pdfMake = (await import('pdfmake/build/pdfmake')).default as any;
  // ponytail: bundled Roboto covers Latin/Cyrillic/Greek only; CJK/Arabic transcripts need a
  // font added to the VFS here.
  pdfMake.addVirtualFileSystem((await import('pdfmake/build/vfs_fonts')).default);

  const text = (rs: Run[]) => rs.map(r => ({ text: r.text, bold: r.bold, italics: r.italic, background: r.code ? '#eeeeee' : undefined }));
  const SIZES = [20, 16, 14, 12, 11, 11];

  const content = toBlocks(markdown).map(b => {
    switch (b.kind) {
      case 'heading': return { text: text(b.runs), fontSize: SIZES[b.level - 1], bold: true, margin: [0, 10, 0, 4] };
      case 'paragraph': return { text: text(b.runs), margin: [0, 0, 0, 6] };
      case 'rule': return { canvas: [{ type: 'line', x1: 0, y1: 0, x2: 515, y2: 0, lineWidth: 0.5, lineColor: '#999999' }], margin: [0, 6, 0, 6] };
      // ponytail: list items are rendered as indented lines with their own marker, which keeps
      // mixed/nested lists flat and simple; switch to pdfmake ul/ol trees if styling needs it.
      case 'item': return { text: [{ text: b.ordered ? `${b.index}. ` : '• ' }, ...text(b.runs)], margin: [12 + b.depth * 14, 0, 0, 3] };
      case 'table': return {
        table: { headerRows: 1, widths: b.header.map(() => '*'), body: [b.header.map(c => ({ text: text(c), bold: true })), ...b.rows.map(r => r.map(c => ({ text: text(c) })))] },
        layout: 'lightHorizontalLines', margin: [0, 4, 0, 8], fontSize: 9,
      };
    }
  });

  const doc = pdfMake.createPdf({ content, defaultStyle: { fontSize: 10.5, lineHeight: 1.25 }, pageMargins: [40, 40, 40, 40] });
  return new Uint8Array(await doc.getBuffer());
}
