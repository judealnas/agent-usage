/** Continuous green (120 degrees) through yellow (60) to red (0). */
export function usageHue(usedPercent: number, redAt: number): number {
  const threshold = Number.isFinite(redAt) && redAt >= 1 && redAt <= 100 ? redAt : 90;
  const used = Number.isFinite(usedPercent) ? usedPercent : 0;
  return 120 * (1 - Math.min(1, Math.max(0, used / threshold)));
}
