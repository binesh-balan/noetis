'use client';

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Button } from './ui/button';
import { Input } from './ui/input';
import { Textarea } from './ui/textarea';

type Source = 'org' | 'custom' | 'builtin';
interface TemplateInfo { id: string; name: string; description: string; source: Source }
interface Section { title: string; instruction: string; format: 'paragraph' | 'list' | 'string'; item_format?: string; example_item_format?: string }
interface Template { name: string; description: string; sections: Section[] }

const SOURCE_LABEL: Record<Source, string> = { org: 'Organization', custom: 'Mine', builtin: 'Built-in' };

const slug = (s: string) => s.toLowerCase().replace(/[^a-z0-9]+/g, '_').replace(/^_|_$/g, '') || 'template';

export function TemplateSettings() {
  const [templates, setTemplates] = useState<TemplateInfo[]>([]);
  // editing: id being saved to + draft; null = list view
  const [editing, setEditing] = useState<{ id: string; isNew: boolean; draft: Template } | null>(null);

  const refresh = useCallback(async () => {
    try {
      setTemplates(await invoke<TemplateInfo[]>('api_list_templates'));
    } catch (e) {
      toast.error(`Failed to load templates: ${e}`);
    }
  }, []);

  useEffect(() => { refresh(); }, [refresh]);

  const uniqueId = (base: string) => {
    let id = base, n = 2;
    while (templates.some(t => t.id === id)) id = `${base}_${n++}`;
    return id;
  };

  const startNew = () => setEditing({
    id: '', isNew: true,
    draft: { name: '', description: '', sections: [{ title: 'Summary', instruction: 'Brief summary of the meeting', format: 'paragraph' }] },
  });

  const startEdit = async (t: TemplateInfo, copy: boolean) => {
    try {
      const draft = await invoke<Template>('api_get_template', { templateId: t.id });
      setEditing(copy
        ? { id: '', isNew: true, draft: { ...draft, name: `${draft.name} (copy)` } }
        : { id: t.id, isNew: false, draft });
    } catch (e) {
      toast.error(`Failed to open template: ${e}`);
    }
  };

  const save = async () => {
    if (!editing) return;
    const id = editing.isNew ? uniqueId(slug(editing.draft.name)) : editing.id;
    try {
      await invoke('api_save_custom_template', { templateId: id, template: editing.draft });
      toast.success('Template saved');
      setEditing(null);
      refresh();
    } catch (e) {
      toast.error(String(e));
    }
  };

  const remove = async (t: TemplateInfo) => {
    if (!confirm(`Delete template "${t.name}"?`)) return;
    try {
      await invoke('api_delete_custom_template', { templateId: t.id });
      refresh();
    } catch (e) {
      toast.error(String(e));
    }
  };

  if (editing) {
    const { draft } = editing;
    const set = (d: Partial<Template>) => setEditing({ ...editing, draft: { ...draft, ...d } });
    const setSection = (i: number, s: Partial<Section>) =>
      set({ sections: draft.sections.map((x, j) => (j === i ? { ...x, ...s } : x)) });

    return (
      <div className="bg-white rounded-lg border border-gray-200 p-6 shadow-sm flex flex-col gap-4">
        <h3 className="text-lg font-semibold">{editing.isNew ? 'New template' : `Edit "${draft.name}"`}</h3>
        <label className="text-sm font-medium">Name
          <Input value={draft.name} onChange={e => set({ name: e.target.value })} placeholder="Client call" />
        </label>
        <label className="text-sm font-medium">Description
          <Input value={draft.description} onChange={e => set({ description: e.target.value })} placeholder="When to use this template" />
        </label>

        <div className="flex flex-col gap-3">
          <p className="text-sm font-medium">Sections</p>
          {draft.sections.map((s, i) => (
            <div key={i} className="rounded-md border border-gray-200 p-3 flex flex-col gap-2">
              <div className="flex gap-2">
                <Input aria-label="Section title" value={s.title} onChange={e => setSection(i, { title: e.target.value })} placeholder="Section title" />
                <select
                  aria-label="Section format"
                  className="rounded-md border border-gray-200 px-2 text-sm"
                  value={s.format}
                  onChange={e => setSection(i, { format: e.target.value as Section['format'] })}
                >
                  <option value="paragraph">Paragraph</option>
                  <option value="list">List</option>
                  <option value="string">Single line</option>
                </select>
                <Button variant="ghost" size="sm" disabled={draft.sections.length === 1}
                  onClick={() => set({ sections: draft.sections.filter((_, j) => j !== i) })}>Remove</Button>
              </div>
              <Textarea aria-label="Section instruction" value={s.instruction} rows={2}
                onChange={e => setSection(i, { instruction: e.target.value })}
                placeholder="What the AI should extract for this section" />
            </div>
          ))}
          <Button variant="outline" size="sm" className="self-start"
            onClick={() => set({ sections: [...draft.sections, { title: '', instruction: '', format: 'list' }] })}>
            Add section
          </Button>
        </div>

        <div className="flex gap-2 justify-end">
          <Button variant="ghost" onClick={() => setEditing(null)}>Cancel</Button>
          <Button onClick={save}>Save</Button>
        </div>
      </div>
    );
  }

  return (
    <div className="bg-white rounded-lg border border-gray-200 p-6 shadow-sm">
      <div className="flex items-center justify-between mb-4">
        <div>
          <h3 className="text-lg font-semibold">Summary Templates</h3>
          <p className="text-sm text-gray-600">Organization templates are managed by IT. Duplicate any template to customize it.</p>
        </div>
        <Button onClick={startNew}>New template</Button>
      </div>
      <ul className="divide-y divide-gray-100">
        {templates.map(t => (
          <li key={t.id} className="py-3 flex items-center justify-between gap-4">
            <div className="min-w-0">
              <p className="font-medium truncate">
                {t.name}
                <span className="ml-2 rounded bg-gray-100 px-1.5 py-0.5 text-xs text-gray-600">{SOURCE_LABEL[t.source]}</span>
              </p>
              <p className="text-sm text-gray-600 truncate">{t.description}</p>
            </div>
            <div className="flex gap-1 shrink-0">
              {t.source === 'custom' && <Button variant="ghost" size="sm" onClick={() => startEdit(t, false)}>Edit</Button>}
              <Button variant="ghost" size="sm" onClick={() => startEdit(t, true)}>Duplicate</Button>
              {t.source === 'custom' && <Button variant="ghost" size="sm" onClick={() => remove(t)}>Delete</Button>}
            </div>
          </li>
        ))}
      </ul>
    </div>
  );
}
