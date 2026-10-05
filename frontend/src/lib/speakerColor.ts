// Literal class strings so Tailwind's JIT picks them up; never build these by concatenation.
const SPEAKER_CLASSES = ['text-speaker-1', 'text-speaker-2', 'text-speaker-3', 'text-speaker-4', 'text-speaker-5', 'text-speaker-6'] as const;

/** Colour class for the speaker at `index` in first-appearance order (wraps after six). */
export function speakerColorClass(index: number): string {
  return SPEAKER_CLASSES[((index % 6) + 6) % 6];
}
