/** Same routing rule the sidebar uses for opening a meeting item. */
export function meetingHref(id: string): string {
  if (id.startsWith('intro-call')) return '/';
  if (id.includes('-')) return `/meeting-details?id=${id}`;
  return `/notes/${id}`;
}
