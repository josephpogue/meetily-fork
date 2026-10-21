'use client';

import { useEffect, useRef, useState, KeyboardEvent } from 'react';
import { Mic, Send, Square } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { useAssistant } from '@/contexts/AssistantContext';
import { isTodoEntry, stripTodoPrefix } from '@/lib/todos';
import { KindChip } from './KindChip';

interface AskBoxProps {
  disabled: boolean;
  /** Shown in the panel's quiet error line, e.g. when a to-do cannot be added. */
  onError?: (message: string) => void;
}

/**
 * Typed ask input pinned at the bottom of the panel, plus the voice ask mic
 * button. Mic click starts capture; a second click submits what was heard.
 * Escape cancels an in-progress capture (handled panel-wide in AssistantPanel,
 * so it works whether or not this input has focus).
 *
 * A value that starts with "todo:" (or Opt-T, an in-app key) turns the box into
 * a to-do entry: the rose To do chip shows in the field and Send or Enter adds
 * the text after the prefix as a to-do instead of asking. Backspace on an empty
 * entry drops back to asking.
 */
export function AskBox({ disabled, onError }: AskBoxProps) {
  const { ask, addTodo, voice, voiceStart, voiceFinish, status } = useAssistant();
  const assistantOn = status.enabled;
  const [value, setValue] = useState('');
  const inputRef = useRef<HTMLInputElement>(null);

  const todoMode = isTodoEntry(value);
  // In to-do mode the chip stands for the prefix, so the field shows only the text after it.
  const shown = todoMode ? stripTodoPrefix(value) : value;
  const text = shown.trim();

  const submit = async () => {
    if (!text) return;
    if (!todoMode) {
      ask(text);
      setValue('');
      return;
    }
    const error = await addTodo(text);
    if (error) onError?.(error);
    else setValue('');
  };

  const handleKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Enter') {
      e.preventDefault();
      void submit();
    } else if (e.key === 'Backspace' && todoMode && shown === '') {
      e.preventDefault();
      setValue('');
    }
  };

  // Opt-T: in-app only, while the panel (and so this box) is shown and the assistant
  // is on. Option alone, not over a modal dialog, and not while another text field
  // has focus; in those cases the key is left alone. preventDefault keeps macOS
  // from typing a dagger.
  useEffect(() => {
    if (!assistantOn) return;
    const onKeyDown = (e: globalThis.KeyboardEvent) => {
      if (!e.altKey || e.metaKey || e.ctrlKey || e.shiftKey || e.code !== 'KeyT') return;
      if (document.querySelector('[role="dialog"], [role="alertdialog"]')) return;
      const active = document.activeElement as HTMLElement | null;
      if (active && active !== inputRef.current) {
        const tag = active.tagName;
        if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || active.isContentEditable) return;
      }
      e.preventDefault();
      setValue((current) => (isTodoEntry(current) ? current : `todo: ${current}`));
      inputRef.current?.focus?.();
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [assistantOn]);

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
        <div
          className={cn(
            'flex h-10 min-w-0 flex-1 items-center gap-2 rounded-lg border bg-white px-3 shadow-sm',
            todoMode ? 'border-rose-300 ring-2 ring-rose-100' : 'border-input focus-within:ring-1 focus-within:ring-ring'
          )}
        >
          {todoMode && <KindChip kind="todo" />}
          <input
            ref={inputRef}
            value={shown}
            onChange={(e) => setValue(todoMode ? `todo: ${e.target.value.replace(/^\s+/, '')}` : e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder={todoMode ? 'What needs doing?' : 'Ask a question, or type todo: to add a to-do'}
            aria-label="Ask a question"
            className="h-full min-w-0 flex-1 bg-transparent text-sm text-gray-900 outline-none placeholder:text-gray-500"
          />
        </div>
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
          onClick={() => void submit()}
          disabled={disabled || !text}
          title={todoMode ? 'Add to-do' : 'Send'}
          aria-label={todoMode ? 'Add to-do' : 'Send question'}
        >
          <Send className="h-4 w-4" />
        </Button>
      </div>
    </div>
  );
}
