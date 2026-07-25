export type PaletteCommand = {
  id: string;
  label: string;
  keywords: readonly string[];
  run(): void;
};

export function filterCommands(
  commands: readonly PaletteCommand[],
  query: string,
): PaletteCommand[] {
  const normalizedQuery = query.trim().toLocaleLowerCase();
  if (normalizedQuery.length === 0) return [...commands];

  return commands.filter(({ label, keywords }) => {
    const searchableText = [label, ...keywords].join(" ").toLocaleLowerCase();
    return searchableText.includes(normalizedQuery);
  });
}

export function nextActiveIndex(
  currentIndex: number,
  direction: "next" | "previous",
  commandCount: number,
): number {
  if (commandCount === 0) return -1;

  const offset = direction === "next" ? 1 : -1;
  return (currentIndex + offset + commandCount) % commandCount;
}
