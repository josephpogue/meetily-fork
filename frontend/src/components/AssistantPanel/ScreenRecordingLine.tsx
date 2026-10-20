'use client';

import { useCallback, useEffect, useState } from 'react';
import { AlertTriangle } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { assistantService } from '@/services/assistantService';

const RECHECK_MS = 30_000;

/**
 * The amber line under the header: the "See the Zoom window" setting is on but
 * macOS has not granted Screen Recording, so no pictures get saved. Checked on
 * mount, on window focus (Joseph grants it in System Settings and comes back),
 * when a recording starts, and every 30 s while the assistant is on. A failing
 * service call hides the line and never breaks the panel.
 */
export function ScreenRecordingLine({ enabled, isRecording }: { enabled: boolean; isRecording: boolean }) {
  const [blocked, setBlocked] = useState(false);

  const check = useCallback(async () => {
    try {
      const [settings, allowed] = await Promise.all([
        assistantService.getSettings(),
        assistantService.screenRecordingAllowed(),
      ]);
      setBlocked(Boolean(settings.seeZoomWindow) && allowed === false);
    } catch {
      setBlocked(false);
    }
  }, []);

  useEffect(() => {
    if (!enabled) {
      setBlocked(false);
      return;
    }
    void check();
    const timer = setInterval(() => void check(), RECHECK_MS);
    const onFocus = () => void check();
    window.addEventListener('focus', onFocus);
    return () => {
      clearInterval(timer);
      window.removeEventListener('focus', onFocus);
    };
  }, [enabled, check]);

  // A recording starting re-checks right away.
  useEffect(() => {
    if (enabled && isRecording) void check();
  }, [enabled, isRecording, check]);

  if (!enabled || !blocked) return null;

  const openSettings = () => {
    assistantService.openScreenRecordingSettings().catch(() => {});
  };

  return (
    <div
      role="status"
      className="flex shrink-0 items-center gap-2 border-b border-amber-200 bg-amber-50 px-3.5 py-2 text-sm text-amber-900"
    >
      <AlertTriangle className="h-4 w-4 shrink-0 text-amber-700" aria-hidden="true" />
      <span className="min-w-0 flex-1">Can&apos;t see Zoom: Screen Recording is off.</span>
      <Button type="button" variant="outline" size="sm" className="h-7 border-amber-300 bg-white px-2.5 text-xs" onClick={openSettings}>
        Allow
      </Button>
    </div>
  );
}
