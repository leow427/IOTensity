import { useEffect } from 'react';
import type { SyncOutput } from '../sync/output';

// The scene renders on demand: a new native color snapshot requests one frame
// instead of a per-frame poll of the passive output cache.
export function useOutputInvalidation(
  output: SyncOutput,
  invalidate: () => void,
) {
  useEffect(
    () => output.subscribeColors(() => invalidate()),
    [output, invalidate],
  );
}
