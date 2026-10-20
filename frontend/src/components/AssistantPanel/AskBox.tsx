'use client';

import { useState, KeyboardEvent } from 'react';
import { Mic, Send, Square } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { useAssistant } from '@/contexts/AssistantContext';

interface AskBoxProps {
  disabled: boolean;
}

/**
 * Typed ask input pinned at the bottom of the panel, plus the voice ask mic
 * button. Mic click starts capture; a second click submits what was heard.
 * Escape cancels an in-progress capture (handled panel-wide in AssistantPanel,
 * so it works whether or not this input has focus).
 */
export function AskBox({ disabled }: AskBoxProps) {
  const { ask, voice, voiceStart, voiceFinish } = useAssistant();
  const [value, setValue] = useState('');

  const submit = () => {
    const text = value.trim();
    if (!text) return;
    ask(text);
    setValue('');
  };

  const handleKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Enter') {
      e.preventDefault();
      submit();
    }
  };

  const isListening = voice.state === 'listening';
  const isSubmitting = voice.state === 'submitting';

  const handleMicClick = () => {
    if (isListening) {
      voiceFinish();
    } else if (!isSubmitting) {
      voiceStart();
    }
  };

  return (
    <div className="p-3">
      {isListening && (
        <p className="mb-2 text-xs italic text-gray-500">
          {voice.heard || 'Listening...'}
        </p>
      )}
      <div className="flex items-center gap-2">
        <Input
          value={value}
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder="Ask a question"
          className="h-10 flex-1 rounded-lg"
        />
        <Button
          type="button"
          variant={isListening ? 'red' : 'outline'}
          size="icon"
          className="h-10 w-10 shrink-0"
          onClick={handleMicClick}
          disabled={disabled || isSubmitting}
          title={isListening ? 'Stop and submit' : 'Voice ask'}
          aria-label={isListening ? 'Stop and submit' : 'Speak a question'}
        >
          {isListening ? <Square className="h-4 w-4" /> : <Mic className="h-4 w-4" />}
        </Button>
        <Button
          type="button"
          variant="outline"
          size="icon"
          className="h-10 w-10 shrink-0"
          onClick={submit}
          disabled={disabled || !value.trim()}
          title="Send"
          aria-label="Send question"
        >
          <Send className="h-4 w-4" />
        </Button>
      </div>
    </div>
  );
}
