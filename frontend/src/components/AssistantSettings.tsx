'use client';

import { useCallback, useEffect, useState } from 'react';
import { toast } from 'sonner';
import { CheckCircle2, XCircle } from 'lucide-react';
import { Switch } from '@/components/ui/switch';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Button } from '@/components/ui/button';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import { assistantService } from '@/services/assistantService';
import { AssistantClaudeProbe, AssistantSettings as AssistantSettingsData } from '@/types';

const DEFAULT_SETTINGS: AssistantSettingsData = {
  enabled: true,
  claudePath: '',
  fastModel: 'claude-sonnet-5',
  fastEffort: 'medium',
  deepModel: 'claude-opus-5',
  deepEffort: 'medium',
  triggerMode: 'gated',
  quietGapSecs: 2.0,
  names: 'joseph,joe',
  vaultRoot: '',
  deepReadDirs: '',
  seeZoomWindow: true,
};

const MODEL_OPTIONS = [
  { value: 'claude-sonnet-5', label: 'Claude Sonnet 5' },
  { value: 'claude-opus-5', label: 'Claude Opus 5' },
  { value: 'claude-haiku-4-5-20251001', label: 'Claude Haiku 4.5' },
];

const EFFORT_OPTIONS = ['low', 'medium', 'high'];

const TRIGGER_MODE_OPTIONS: { value: AssistantSettingsData['triggerMode']; label: string }[] = [
  { value: 'manual', label: 'Manual' },
  { value: 'gated', label: 'Gated' },
  { value: 'continuous', label: 'Continuous' },
];

/**
 * Assistant settings tab, modeled on SummaryModelSettings. Loads via
 * assistant_get_settings, saves via assistant_save_settings.
 */
export function AssistantSettings() {
  const [settings, setSettings] = useState<AssistantSettingsData>(DEFAULT_SETTINGS);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);
  const [probe, setProbe] = useState<AssistantClaudeProbe | null>(null);
  const [screenAllowed, setScreenAllowed] = useState(true);

  const loadSettings = useCallback(async () => {
    try {
      const loaded = await assistantService.getSettings();
      setSettings(loaded);
    } catch (error) {
      console.error('[AssistantSettings] Failed to load assistant settings:', error);
    } finally {
      setLoading(false);
    }
  }, []);

  const checkScreenPermission = useCallback(async () => {
    try {
      setScreenAllowed(await assistantService.screenRecordingAllowed());
    } catch (error) {
      console.error('[AssistantSettings] Failed to check Screen Recording permission:', error);
    }
  }, []);

  useEffect(() => {
    loadSettings();
    checkScreenPermission();
    // Joseph grants the permission in System Settings, then comes back here.
    window.addEventListener('focus', checkScreenPermission);
    return () => window.removeEventListener('focus', checkScreenPermission);
  }, [loadSettings, checkScreenPermission]);

  const handleOpenScreenSettings = async () => {
    try {
      await assistantService.openScreenRecordingSettings();
    } catch (error) {
      console.error('[AssistantSettings] Failed to open System Settings:', error);
      toast.error('Could not open System Settings', {
        description: error instanceof Error ? error.message : String(error),
      });
    }
  };

  const handleSave = async () => {
    setSaving(true);
    try {
      await assistantService.saveSettings(settings);
      toast.success('Assistant settings saved');
    } catch (error) {
      console.error('[AssistantSettings] Failed to save assistant settings:', error);
      toast.error('Failed to save assistant settings', {
        description: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setSaving(false);
    }
  };

  const handleTestClaude = async () => {
    setTesting(true);
    setProbe(null);
    try {
      const result = await assistantService.testClaude();
      setProbe(result);
    } catch (error) {
      console.error('[AssistantSettings] Failed to test claude binary:', error);
      setProbe({
        ok: false,
        version: '',
        error: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setTesting(false);
    }
  };

  const update = <K extends keyof AssistantSettingsData>(key: K, value: AssistantSettingsData[K]) => {
    setSettings((prev) => ({ ...prev, [key]: value }));
  };

  if (loading) {
    return (
      <div className="bg-white rounded-lg border border-gray-200 p-6 shadow-sm">
        <p className="text-sm text-gray-500">Loading assistant settings...</p>
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-4">
      <div className="bg-white rounded-lg border border-gray-200 p-6 shadow-sm">
        <div className="flex items-center justify-between">
          <div>
            <h3 className="text-lg font-semibold text-gray-900 mb-2">Live Assistant</h3>
            <p className="text-sm text-gray-600">
              Turn the in-meeting assistant on or off. Runs entirely through your Claude subscription.
            </p>
          </div>
          <Switch checked={settings.enabled} onCheckedChange={(v) => update('enabled', v)} />
        </div>
      </div>

      <div className="bg-white rounded-lg border border-gray-200 p-6 shadow-sm">
        <div className="flex items-center justify-between">
          <div>
            <Label htmlFor="see-zoom-window" className="text-base font-semibold text-gray-900">See the Zoom or Teams window</Label>
            <p className="text-sm text-gray-600 mt-1">
              Sends changed pictures of the Zoom or Teams meeting window to Claude with each answer.
            </p>
          </div>
          <Switch
            id="see-zoom-window"
            checked={settings.seeZoomWindow}
            onCheckedChange={(v) => update('seeZoomWindow', v)}
          />
        </div>
        {settings.seeZoomWindow && !screenAllowed && (
          <div className="mt-3 flex items-center justify-between gap-3 text-sm text-amber-700">
            <span>Allow Screen Recording in System Settings</span>
            <Button variant="outline" size="sm" onClick={handleOpenScreenSettings}>
              Open System Settings
            </Button>
          </div>
        )}
      </div>

      <div className="bg-white rounded-lg border border-gray-200 p-6 shadow-sm space-y-6">
        <div>
          <h3 className="text-lg font-semibold mb-1">Claude CLI</h3>
          <p className="text-sm text-gray-600 mb-4">
            Leave the path blank to auto-detect the claude binary.
          </p>
          <Label htmlFor="claude-path" className="mb-1.5 block">Claude path (optional)</Label>
          <div className="flex gap-2">
            <Input
              id="claude-path"
              value={settings.claudePath}
              onChange={(e) => update('claudePath', e.target.value)}
              placeholder="/opt/homebrew/bin/claude"
              className="flex-1"
            />
            <Button variant="outline" onClick={handleTestClaude} disabled={testing}>
              {testing ? 'Testing...' : 'Test claude'}
            </Button>
          </div>
          {probe && (
            <div className={`mt-2 flex items-center gap-1.5 text-sm ${probe.ok ? 'text-green-600' : 'text-red-600'}`}>
              {probe.ok ? <CheckCircle2 className="h-4 w-4" /> : <XCircle className="h-4 w-4" />}
              {probe.ok ? probe.version || 'Claude CLI is available' : probe.error || 'Claude CLI is not available'}
            </div>
          )}
        </div>

        <div className="grid grid-cols-2 gap-4">
          <div>
            <Label className="mb-1.5 block">Fast lane model</Label>
            <Select value={settings.fastModel} onValueChange={(v) => update('fastModel', v)}>
              <SelectTrigger><SelectValue /></SelectTrigger>
              <SelectContent>
                {MODEL_OPTIONS.map((m) => (
                  <SelectItem key={m.value} value={m.value}>{m.label}</SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div>
            <Label className="mb-1.5 block">Fast lane effort</Label>
            <Select value={settings.fastEffort} onValueChange={(v) => update('fastEffort', v)}>
              <SelectTrigger><SelectValue /></SelectTrigger>
              <SelectContent>
                {EFFORT_OPTIONS.map((e) => (
                  <SelectItem key={e} value={e} className="capitalize">{e}</SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div>
            <Label className="mb-1.5 block">Deep lane model</Label>
            <Select value={settings.deepModel} onValueChange={(v) => update('deepModel', v)}>
              <SelectTrigger><SelectValue /></SelectTrigger>
              <SelectContent>
                {MODEL_OPTIONS.map((m) => (
                  <SelectItem key={m.value} value={m.value}>{m.label}</SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div>
            <Label className="mb-1.5 block">Deep lane effort</Label>
            <Select value={settings.deepEffort} onValueChange={(v) => update('deepEffort', v)}>
              <SelectTrigger><SelectValue /></SelectTrigger>
              <SelectContent>
                {EFFORT_OPTIONS.map((e) => (
                  <SelectItem key={e} value={e} className="capitalize">{e}</SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        </div>

        <div className="grid grid-cols-2 gap-4">
          <div>
            <Label className="mb-1.5 block">Default trigger mode</Label>
            <Select value={settings.triggerMode} onValueChange={(v) => update('triggerMode', v as AssistantSettingsData['triggerMode'])}>
              <SelectTrigger><SelectValue /></SelectTrigger>
              <SelectContent>
                {TRIGGER_MODE_OPTIONS.map((m) => (
                  <SelectItem key={m.value} value={m.value}>{m.label}</SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div>
            <Label htmlFor="quiet-gap" className="mb-1.5 block">Quiet gap (seconds)</Label>
            <Input
              id="quiet-gap"
              type="number"
              step={0.5}
              min={0}
              value={settings.quietGapSecs}
              onChange={(e) => update('quietGapSecs', parseFloat(e.target.value) || 0)}
            />
          </div>
        </div>

        <div>
          <Label htmlFor="names" className="mb-1.5 block">Names</Label>
          <Input
            id="names"
            value={settings.names}
            onChange={(e) => update('names', e.target.value)}
            placeholder="joseph,joe"
          />
          <p className="mt-1 text-xs text-gray-500">Names the room calls you, comma separated.</p>
        </div>

        <div>
          <Label htmlFor="vault-root" className="mb-1.5 block">Vault root (optional)</Label>
          <Input
            id="vault-root"
            value={settings.vaultRoot}
            onChange={(e) => update('vaultRoot', e.target.value)}
            placeholder="~/brain/wiki"
          />
          <p className="mt-1 text-xs text-gray-500">Where end-of-meeting notes get saved. Blank uses the default.</p>
        </div>

        <div className="flex justify-end">
          <Button onClick={handleSave} disabled={saving}>
            {saving ? 'Saving...' : 'Save settings'}
          </Button>
        </div>
      </div>
    </div>
  );
}
