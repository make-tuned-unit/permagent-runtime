import { useEffect, useState } from 'react';
import { api } from '../../lib/api';
import { IDENTITY_CHAIN_KEY, IDENTITY_CHITIN_SLUG_KEY, type IdentityChainWire } from '../../config/chain';
import { useIdentityStore } from '../../stores/identityStore';
import { useTheme } from '../../styles/useTheme';
import { space, textSize } from '../../styles/tokens';
import { Row, Section, SaveButton, TextInput } from './atoms';

const ADDRESS_RE = /^0x[0-9a-fA-F]{40}$/;
const SLUG_RE = /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/;
const ID_RE = /^\d+$/;

interface Fields {
  slug: string;
  sbt: string;
  passport: string;
  owner: string;
  sbtId: string;
  passportId: string;
}

const EMPTY: Fields = { slug: '', sbt: '', passport: '', owner: '', sbtId: '', passportId: '' };

/** Returns an error string, or null when the fields are acceptable to save. */
export function validateIdentityFields(f: Fields): string | null {
  if (f.slug.trim() && !SLUG_RE.test(f.slug.trim())) return 'The chitin.id slug may only use letters, digits, dots, dashes and underscores.';
  const chainFilled = [f.sbt, f.passport, f.owner, f.sbtId, f.passportId].some((v) => v.trim());
  if (!chainFilled) return null;
  if (![f.sbt, f.passport, f.owner].every((v) => ADDRESS_RE.test(v.trim()))) return 'Contract and owner addresses must be 0x followed by 40 hex characters.';
  if (!ID_RE.test(f.sbtId.trim()) || !ID_RE.test(f.passportId.trim())) return 'Token ids must be whole numbers.';
  return null;
}

/**
 * Settings, Persona: the optional on-chain identity. Both config keys are empty
 * by default; until a user fills them in, nothing contacts chitin.id or any RPC
 * and the passport shows a neutral "not linked" state.
 */
export function OnChainIdentitySection() {
  const { colors } = useTheme();
  const [f, setF] = useState<Fields>(EMPTY);
  const [dirty, setDirty] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    Promise.all([
      api.readConfig(IDENTITY_CHITIN_SLUG_KEY).catch(() => null),
      api.readConfig(IDENTITY_CHAIN_KEY).catch(() => null),
    ]).then(([slug, chain]) => {
      if (cancelled) return;
      const c = (chain && typeof chain === 'object' ? chain : {}) as IdentityChainWire;
      setF({
        slug: typeof slug === 'string' ? slug : '',
        sbt: c.sbt_contract ?? '',
        passport: c.passport_contract ?? '',
        owner: c.owner ?? '',
        sbtId: c.sbt_token_id != null ? String(c.sbt_token_id) : '',
        passportId: c.passport_token_id != null ? String(c.passport_token_id) : '',
      });
    });
    return () => { cancelled = true; };
  }, []);

  const set = (k: keyof Fields) => (v: string) => { setF((p) => ({ ...p, [k]: v })); setDirty(true); setError(null); };

  const save = async () => {
    const problem = validateIdentityFields(f);
    if (problem) { setError(problem); return; }
    setSaving(true);
    try {
      const chain: IdentityChainWire = f.sbt.trim()
        ? {
            sbt_contract: f.sbt.trim(), passport_contract: f.passport.trim(), owner: f.owner.trim(),
            sbt_token_id: f.sbtId.trim(), passport_token_id: f.passportId.trim(),
          }
        : {};
      await api.upsertConfig('identity_chitin_slug', f.slug.trim());
      await api.upsertConfig('identity_chain', chain);
      setDirty(false);
      void useIdentityStore.getState().refresh();
    } catch {
      setError('Could not save the identity settings.');
    } finally {
      setSaving(false);
    }
  };

  return (
    <Section
      title="On-chain identity (optional)"
      sub="Link your agent to a chitin.id profile and the contracts you own. Leave these empty to keep everything local: nothing is looked up until you fill them in."
    >
      <Row label="chitin.id slug" hint="The profile name after chitin.id/.">
        <TextInput mono value={f.slug} onChange={set('slug')} placeholder="your-agent" />
      </Row>
      <Row label="Soul token contract" hint="Address of your soulbound token contract on Base.">
        <TextInput mono value={f.sbt} onChange={set('sbt')} placeholder="0x…" />
      </Row>
      <Row label="Soul token id"><TextInput mono value={f.sbtId} onChange={set('sbtId')} /></Row>
      <Row label="Passport contract" hint="Address of your agent passport (registry) contract on Base.">
        <TextInput mono value={f.passport} onChange={set('passport')} placeholder="0x…" />
      </Row>
      <Row label="Passport token id"><TextInput mono value={f.passportId} onChange={set('passportId')} /></Row>
      <Row label="Owner wallet" hint="The address that should own both tokens.">
        <TextInput mono value={f.owner} onChange={set('owner')} placeholder="0x…" />
      </Row>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'flex-end', gap: space.xl }}>
        {error && <span role="alert" style={{ fontSize: textSize.caption, color: colors.danger }}>{error}</span>}
        <SaveButton onClick={() => { void save(); }} disabled={!dirty || saving} saving={saving} />
      </div>
    </Section>
  );
}
