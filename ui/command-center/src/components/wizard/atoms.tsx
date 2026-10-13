import { useState, useRef, useEffect, useLayoutEffect, useId, type CSSProperties, type ReactNode, type KeyboardEvent as ReactKeyboardEvent } from 'react';
import { FiArrowLeft, FiChevronDown, FiCheck } from 'react-icons/fi';
import { font, ease, duration, radius, space, textSize, type } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';
import { useForumReducedMotion } from '../world/forum/useForumReducedMotion';

// ── WizardHeading / WizardSubhead ──────────────────────────────────────
// One heading scale for every step, so the flow reads as one designed object.
// `tabIndex={-1}` lets a step move focus to its title when it has no field.
export function WizardHeading({ children, id, style = {} }: { children: ReactNode; id?: string; style?: CSSProperties }) {
  const { colors } = useTheme();
  return (
    <h1 id={id} tabIndex={-1} data-step-heading="" style={{
      fontFamily: font.display, ...type.display, color: colors.text, margin: 0,
      textAlign: 'center', outline: 'none',
      ...style,
    }}>{children}</h1>
  );
}

export function WizardSubhead({ children, style = {} }: { children: ReactNode; style?: CSSProperties }) {
  const { colors } = useTheme();
  return (
    <p style={{
      fontFamily: font.body, fontSize: textSize.body, lineHeight: 1.55, color: colors.textMuted,
      margin: `${space.md}px auto 0`, textAlign: 'center', maxWidth: 460,
      ...style,
    }}>{children}</p>
  );
}

/** Small uppercase section label. */
export function FieldLabel({ children, htmlFor, id }: { children: ReactNode; htmlFor?: string; id?: string }) {
  const { colors } = useTheme();
  return (
    <label htmlFor={htmlFor} id={id} style={{
      ...type.label, fontFamily: font.body, color: colors.textMuted,
      display: 'block', marginBottom: space.sm,
    }}>{children}</label>
  );
}

/** Inline result line: success / warning / error / neutral, announced. */
export function StatusLine({ tone, children, style = {} }: {
  tone: 'ok' | 'warn' | 'error' | 'info'; children: ReactNode; style?: CSSProperties;
}) {
  const { colors, reduceMotion } = useTheme();
  const ref = useRef<HTMLDivElement>(null);
  // A result is the answer to what the person just pressed: bring it into view
  // when it lands below the fold of a scrolled step.
  useEffect(() => {
    if (tone === 'info') return;
    ref.current?.scrollIntoView?.({ block: 'nearest', behavior: reduceMotion ? 'auto' : 'smooth' });
  }, [tone, children, reduceMotion]);
  const color = tone === 'ok' ? colors.success : tone === 'warn' ? colors.warning : tone === 'error' ? colors.danger : colors.textMuted;
  return (
    <div ref={ref} role={tone === 'error' ? 'alert' : 'status'} style={{
      fontFamily: font.body, fontSize: textSize.small, lineHeight: 1.5, color,
      display: 'flex', gap: space.sm, alignItems: 'flex-start', ...style,
    }}>
      {tone === 'ok' && <FiCheck size={14} style={{ flexShrink: 0, marginTop: space.xxs }} aria-hidden />}
      <span>{children}</span>
    </div>
  );
}

// ── PrimaryButton ──────────────────────────────────────────────────────
export function PrimaryButton({
  children, disabled, onClick, full, type: buttonType = 'button', pending, autoFocus, style = {} }: {
  children: ReactNode; disabled?: boolean; onClick?: () => unknown; full?: boolean;
  type?: 'button' | 'submit'; pending?: boolean; autoFocus?: boolean; style?: CSSProperties;
}) {
  const { colors } = useTheme();
  // The glow is the one hover effect the primitive's custom properties cannot
  // carry (there is no `--pa-btn-shadow`), so a hover flag drives only that.
  const [hover, setHover] = useState(false);
  return (
    <Button
      colors={colors}
      variant="primary"
      type={buttonType}
      onClick={onClick}
      disabled={disabled}
      pending={pending}
      {...(autoFocus ? { 'data-autofocus': '' } : {})}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      style={{
        '--pa-btn-fg': disabled ? colors.textDim : colors.textOnAccent,
        '--pa-btn-bg': disabled ? colors.purpleSoft : colors.purple,
        '--pa-btn-bg-hover': colors.purpleBright,
        '--pa-btn-bg-active': colors.purple,
        '--pa-btn-border': 'transparent',
        '--pa-btn-border-hover': 'transparent',
        '--pa-btn-pad': `${space.lg}px ${space.xxxl}px`,
        '--pa-btn-radius': `${radius.md}px`,
        '--pa-btn-weight': 600,
        fontFamily: font.body, fontSize: textSize.body, letterSpacing: '-0.01em',
        minHeight: 40, minWidth: 132, width: full ? '100%' : 'auto',
        boxShadow: disabled ? 'none' : hover
          ? `0 0 0 4px ${colors.purpleSoft}, 0 8px 24px ${colors.purpleGlow}`
          : `0 4px 14px ${colors.purpleGlow}`,
        ...style,
      } as CSSProperties}
    >{children}</Button>
  );
}

// ── SecondaryButton (hairline) ─────────────────────────────────────────
export function SecondaryButton({ children, onClick, disabled, style = {} }: {
  children: ReactNode; onClick?: () => unknown; disabled?: boolean; style?: CSSProperties;
}) {
  const { colors } = useTheme();
  return (
    <Button
      colors={colors}
      variant="ghost"
      type="button"
      onClick={onClick}
      disabled={disabled}
      onErrorFeedback={false}
      style={{
        '--pa-btn-bg': 'transparent',
        '--pa-btn-fg': colors.text,
        '--pa-btn-border': colors.border,
        '--pa-btn-bg-hover': colors.fillHover,
        '--pa-btn-border-hover': colors.borderHi,
        '--pa-btn-pad': `${space.md}px ${space.xl}px`,
        '--pa-btn-radius': `${radius.md}px`,
        '--pa-btn-weight': 500,
        fontFamily: font.body, fontSize: textSize.small, minHeight: 36, whiteSpace: 'nowrap',
        ...style,
      } as CSSProperties}
    >{children}</Button>
  );
}

// ── GhostLink ──────────────────────────────────────────────────────────
export function GhostLink({ children, onClick, style = {}, ariaLabel }: {
  children: ReactNode; onClick?: () => unknown; style?: CSSProperties; ariaLabel?: string;
}) {
  const { colors } = useTheme();
  return (
    <Button
      colors={colors}
      variant="bare"
      type="button"
      onClick={onClick}
      aria-label={ariaLabel}
      flashSuccess={false}
      style={{
        '--pa-btn-fg': colors.textMuted,
        '--pa-btn-fg-hover': colors.text,
        '--pa-btn-bg-hover': 'transparent',
        '--pa-btn-bg-active': 'transparent',
        '--pa-btn-pad': `${space.sm}px ${space.xxs}px`,
        '--pa-btn-radius': '0',
        '--pa-btn-weight': 500,
        fontFamily: font.body, fontSize: textSize.small, lineHeight: 1.5,
        ...style,
      } as CSSProperties}
    >{children}</Button>
  );
}

// ── Input ──────────────────────────────────────────────────────────────
export function Input({
  value, onChange, placeholder, type: inputType = 'text', onKeyDown, onBlur, autoFocus, ariaLabel,
  id, describedBy, invalid, maxLength, autoComplete = 'off', style = {} }: {
  value: string; onChange: (v: string) => void; placeholder?: string; type?: string;
  onKeyDown?: (e: ReactKeyboardEvent<HTMLInputElement>) => void;
  onBlur?: () => void; autoFocus?: boolean; ariaLabel?: string; id?: string;
  describedBy?: string; invalid?: boolean; maxLength?: number; autoComplete?: string; style?: CSSProperties;
}) {
  const { colors } = useTheme();
  const [focus, setFocus] = useState(false);
  return (
    <input type={inputType} value={value} onChange={e => onChange(e.target.value)}
      placeholder={placeholder} spellCheck={false} id={id}
      {...(autoFocus ? { 'data-autofocus': '' } : {})}
      aria-label={ariaLabel} aria-describedby={describedBy} aria-invalid={invalid || undefined}
      onKeyDown={onKeyDown} maxLength={maxLength} autoComplete={autoComplete}
      onFocus={() => setFocus(true)} onBlur={() => { setFocus(false); onBlur?.(); }}
      style={{
        width: '100%', fontFamily: font.body, fontSize: textSize.body, fontWeight: 400,
        color: colors.text,
        background: colors.inputBg,
        border: `1px solid ${invalid ? colors.danger : focus ? colors.cyan : colors.border}`,
        borderRadius: radius.md, padding: `${space.lg}px ${space.xl}px`, outline: 'none',
        boxShadow: focus ? `0 0 0 3px ${colors.cyanGlow}` : 'none',
        transition: `border-color ${duration.fast}ms ${ease.out}, box-shadow ${duration.fast}ms ${ease.out}`, ...style,
      }}
    />
  );
}

// ── Textarea ───────────────────────────────────────────────────────────
export function Textarea({
  value, onChange, placeholder, rows = 3, id, ariaLabel, style = {} }: {
  value: string; onChange: (v: string) => void; placeholder?: string; rows?: number;
  id?: string; ariaLabel?: string; style?: CSSProperties;
}) {
  const { colors } = useTheme();
  const [focus, setFocus] = useState(false);
  return (
    <textarea value={value} onChange={e => onChange(e.target.value)} id={id} aria-label={ariaLabel}
      placeholder={placeholder} rows={rows} spellCheck
      onFocus={() => setFocus(true)} onBlur={() => setFocus(false)}
      style={{
        width: '100%', fontFamily: font.body, fontSize: textSize.body, fontWeight: 400,
        color: colors.text, background: colors.inputBg, resize: 'none', lineHeight: 1.55,
        border: `1px solid ${focus ? colors.cyan : colors.border}`,
        borderRadius: radius.md, padding: `${space.lg}px ${space.xl}px`, outline: 'none',
        boxShadow: focus ? `0 0 0 3px ${colors.cyanGlow}` : 'none',
        transition: `border-color ${duration.fast}ms ${ease.out}, box-shadow ${duration.fast}ms ${ease.out}`, ...style,
      }}
    />
  );
}

// ── Select ─────────────────────────────────────────────────────────────
export interface SelectOption { value: string; label: string; note?: string }

/**
 * A listbox with the keyboard contract people expect from a native select:
 * ArrowUp/ArrowDown/Space/Enter open it from the trigger, arrows move through
 * the options, Home/End jump, Enter/Space choose, Escape closes without
 * leaving the step (the step's own Escape = Back is not triggered), Tab closes.
 */
export function Select({
  value, onChange, options, placeholder = 'Choose…', ariaLabel, id, autoFocus, style = {} }: {
  value: string; onChange: (v: string) => void; options: SelectOption[];
  placeholder?: string; ariaLabel?: string; id?: string; autoFocus?: boolean; style?: CSSProperties;
}) {
  const { colors } = useTheme();
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const optionRefs = useRef<Array<HTMLDivElement | null>>([]);
  const listId = useId();
  useEffect(() => {
    const onDoc = (e: MouseEvent) => { if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false); };
    document.addEventListener('mousedown', onDoc);
    return () => document.removeEventListener('mousedown', onDoc);
  }, []);
  const current = options.find(o => o.value === value);
  const selectedIndex = Math.max(0, options.findIndex(o => o.value === value));

  // Layout effect: focus lands on the option before the next key is handled,
  // so a fast ArrowDown-then-Enter chooses instead of re-toggling the trigger.
  useLayoutEffect(() => {
    if (open) optionRefs.current[selectedIndex]?.focus();
  }, [open, selectedIndex]);

  const close = (refocus: boolean) => {
    setOpen(false);
    if (refocus) triggerRef.current?.focus();
  };
  const choose = (v: string) => { onChange(v); close(true); };
  const focusAt = (i: number) => {
    const n = options.length;
    if (!n) return;
    optionRefs.current[(i + n) % n]?.focus();
  };

  const onTriggerKey = (e: ReactKeyboardEvent<HTMLButtonElement>) => {
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      setOpen(true);
    }
  };
  const onOptionKey = (e: ReactKeyboardEvent<HTMLDivElement>, i: number) => {
    switch (e.key) {
      case 'ArrowDown': e.preventDefault(); focusAt(i + 1); break;
      case 'ArrowUp': e.preventDefault(); focusAt(i - 1); break;
      case 'Home': e.preventDefault(); focusAt(0); break;
      case 'End': e.preventDefault(); focusAt(options.length - 1); break;
      case 'Enter': case ' ': e.preventDefault(); choose(options[i].value); break;
      case 'Escape': e.preventDefault(); e.stopPropagation(); close(true); break;
      case 'Tab': setOpen(false); break;
      default: break;
    }
  };

  return (
    <div ref={ref} style={{ position: 'relative', ...style }}>
      {/* A listbox trigger, not an action: it keeps aria-haspopup/expanded and
          takes the shared `.pa-btn` interaction rules. */}
      <button
        ref={triggerRef}
        id={id}
        type="button"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={open ? listId : undefined}
        aria-label={ariaLabel}
        {...(autoFocus ? { 'data-autofocus': '' } : {})}
        className="pa-btn"
        onClick={() => setOpen(x => !x)}
        onKeyDown={onTriggerKey}
        style={{
          '--pa-btn-bg': colors.inputBg,
          '--pa-btn-fg': current ? colors.text : colors.textMuted,
          '--pa-btn-border': open ? colors.cyan : colors.border,
          '--pa-btn-bg-hover': colors.inputBg,
          '--pa-btn-border-hover': open ? colors.cyan : colors.borderHi,
          '--pa-btn-bg-active': colors.inputBg,
          '--pa-btn-pad': `${space.lg}px ${space.xl}px`,
          '--pa-btn-radius': `${radius.md}px`,
          '--pa-btn-weight': 500,
          width: '100%', justifyContent: 'space-between',
          fontFamily: font.body, fontSize: textSize.body, lineHeight: 1.5,
          boxShadow: open ? `0 0 0 3px ${colors.cyanGlow}` : 'none',
        } as CSSProperties}
      >
        <span>{current ? current.label : placeholder}</span>
        <FiChevronDown size={12} aria-hidden style={{ opacity: 0.6, transform: open ? 'rotate(180deg)' : 'none', transition: `transform ${duration.base}ms ${ease.out}` }} />
      </button>
      {open && (
        <div id={listId} role="listbox" aria-label={ariaLabel} style={{
          position: 'absolute', top: `calc(100% + ${space.sm}px)`, left: 0, right: 0,
          maxHeight: 260, overflowY: 'auto',
          background: colors.surface, border: `1px solid ${colors.border}`,
          borderRadius: radius.md, padding: space.xs, boxShadow: colors.elevationRaised, zIndex: 30,
        }}>
          {options.map((o, i) => {
            const selected = o.value === value;
            return (
              <div
                key={o.value}
                ref={el => { optionRefs.current[i] = el; }}
                role="option"
                aria-selected={selected}
                tabIndex={-1}
                onClick={() => choose(o.value)}
                onKeyDown={e => onOptionKey(e, i)}
                className="pa-wizard-option"
                style={{
                  display: 'flex', alignItems: 'center', gap: space.lg, padding: `${space.md}px ${space.lg}px`, borderRadius: radius.sm,
                  fontFamily: font.body, fontSize: textSize.small, color: colors.text,
                  background: selected ? colors.cyanSoft : 'transparent', cursor: 'pointer', outline: 'none',
                }}
                onMouseEnter={e => { e.currentTarget.focus(); }}
                onFocus={e => { if (!selected) e.currentTarget.style.background = colors.fillHover; }}
                onBlur={e => { if (!selected) e.currentTarget.style.background = 'transparent'; }}
              >
                <span style={{ flex: 1 }}>{o.label}</span>
                {o.note && <span style={{ fontSize: textSize.micro, color: colors.textMuted }}>{o.note}</span>}
                {selected && <FiCheck size={12} aria-hidden color={colors.cyan} />}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

// ── ChoiceCard: a radio rendered as a card ─────────────────────────────
export function ChoiceCard({ name, value, checked, onSelect, title, detail, autoFocus, disabled = false, status }: {
  name: string; value: string; checked: boolean; onSelect: (v: string) => void;
  title: string; detail: string; autoFocus?: boolean; disabled?: boolean; status?: string;
}) {
  const { colors } = useTheme();
  const [focus, setFocus] = useState(false);
  const [keyboard, setKeyboard] = useState(false);
  const reducedMotion = useForumReducedMotion();
  return (
    <label data-choice-selected={checked} data-choice-focus={focus && keyboard} onPointerDown={() => setKeyboard(false)} style={{
      display: 'flex', gap: space.lg, alignItems: 'flex-start', cursor: disabled ? 'not-allowed' : 'pointer',
      padding: `${space.xl}px ${space.xl}px`, borderRadius: radius.lg,
      background: checked ? colors.cyanSoft : colors.inputBg,
      border: `1px solid ${checked ? colors.cyan : colors.border}`,
      // A neutral offset ring means keyboard focus; accent fill + the checked
      // radio mean selection. Moving focus alone never paints a selected card.
      boxShadow: focus && keyboard ? `0 0 0 ${space.xxs}px ${colors.bg}, 0 0 0 ${space.xs}px ${colors.text}` : 'none',
      transition: reducedMotion || keyboard ? 'none' : `background ${duration.fast}ms ${ease.out}, border-color ${duration.fast}ms ${ease.out}`,
    }}>
      <input
        type="radio" name={name} value={value} checked={checked} disabled={disabled}
        onChange={() => onSelect(value)}
        onFocus={event => { setFocus(true); setKeyboard(event.currentTarget.matches(':focus-visible')); }} onBlur={() => setFocus(false)}
        {...(autoFocus ? { 'data-autofocus': '' } : {})}
        style={{ accentColor: colors.cyan, marginTop: space.xs, flexShrink: 0, outline: 'none' }}
      />
      <span style={{ display: 'flex', flexDirection: 'column', gap: space.xxs }}>
        <span style={{ fontFamily: font.body, fontSize: textSize.body, fontWeight: 600, color: colors.text }}>{title}</span>
        <span style={{ fontFamily: font.body, fontSize: textSize.small, color: colors.textMuted, lineHeight: 1.45 }}>{detail}</span>
        {status !== undefined && <span style={{ fontSize: textSize.caption, color: colors.textMuted, minHeight: '2.9em', lineHeight: 1.45, marginTop: space.sm }}>{status}</span>}
      </span>
    </label>
  );
}

// ── Panel: a quiet bordered surface (content, not glass) ───────────────
export function Panel({ children, style = {}, tone = 'plain' }: {
  children: ReactNode; style?: CSSProperties; tone?: 'plain' | 'accent';
}) {
  const { colors } = useTheme();
  return (
    <div style={{
      padding: space.xxl, borderRadius: radius.lg,
      background: tone === 'accent' ? colors.cyanWash : colors.fillSubtle,
      border: `1px solid ${tone === 'accent' ? colors.borderHi : colors.border}`,
      ...style,
    }}>{children}</div>
  );
}

// ── BackChevron ────────────────────────────────────────────────────────
export function BackChevron({ onClick, disabled }: { onClick: () => void; disabled?: boolean }) {
  const { colors } = useTheme();
  return (
    <Button
      colors={colors}
      variant="bare"
      type="button"
      onClick={onClick}
      disabled={disabled}
      flashSuccess={false}
      aria-label="Back to the previous step"
      style={{
        '--pa-btn-fg': colors.textMuted,
        '--pa-btn-fg-hover': colors.text,
        '--pa-btn-bg-hover': colors.fillHover,
        '--pa-btn-bg-active': colors.fillActive,
        '--pa-btn-pad': `${space.sm}px ${space.lg}px ${space.sm}px ${space.sm}px`,
        '--pa-btn-radius': `${radius.md}px`,
        '--pa-btn-weight': 500,
        fontFamily: font.body, fontSize: textSize.small, lineHeight: 1.5,
      } as CSSProperties}
    >
      <span style={{ display: 'inline-flex', alignItems: 'center', gap: space.sm }}>
        <FiArrowLeft size={14} aria-hidden />
        Back
      </span>
    </Button>
  );
}

// ── StepProgress ───────────────────────────────────────────────────────
/** "Step 2 of 6 · Model" plus one segment per step; announced politely. */
export function StepProgress({ labels, current }: { labels: readonly string[]; current: number }) {
  const { colors } = useTheme();
  const text = `Step ${current + 1} of ${labels.length} · ${labels[current]}`;
  return (
    <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', gap: space.sm }}>
      <div aria-live="polite" style={{ fontFamily: font.body, fontSize: textSize.caption, fontWeight: 600, color: colors.textMuted, letterSpacing: '0.01em' }}>
        {text}
      </div>
      <div aria-hidden style={{ display: 'flex', gap: space.xs }}>
        {labels.map((label, i) => (
          <div key={label} style={{
            width: 28, height: 4, borderRadius: radius.pill,
            background: i < current ? colors.cyan : i === current ? colors.cyan : colors.fillActive,
            opacity: i < current ? 0.45 : 1,
            transition: `background ${duration.slow}ms ${ease.out}`,
          }} />
        ))}
      </div>
    </div>
  );
}
