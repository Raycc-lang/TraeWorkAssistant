import { describe, expect, it } from 'vitest';
import {
  FingerprintSampleParseError,
  parsePrefix,
  parseSystemText,
  sampleTier,
} from './fingerprintSamples';

describe('fingerprintSamples', () => {
  it('parses system text before tools section', () => {
    const s =
      'system="You are opencode, an interactive CLI tool that helps users with software engineering tasks." tools=[\\"bash\\",\\"read\\"]';
    expect(parseSystemText(s)).toBe(
      'You are opencode, an interactive CLI tool that helps users with software engineering tasks.',
    );
  });

  it('parses system text when tools section comes first', () => {
    const s = 'tools=[\\"bash\\"] system="You are opencode."';
    expect(parseSystemText(s)).toBe('You are opencode.');
  });

  it('prefix cuts at first comma and keeps whole sentence before it', () => {
    const s =
      'system="You are opencode, an interactive CLI tool that helps users with software engineering tasks." tools=[\\"bash\\",\\"read\\"]';
    expect(parsePrefix(s)).toBe('You are opencode');
  });

  it('prefix cuts at period when no comma', () => {
    expect(parsePrefix('system="You are opencode."')).toBe('You are opencode');
  });

  it('throws when no system section', () => {
    expect(() => parsePrefix('tools=[\\"bash\\"]')).toThrow(FingerprintSampleParseError);
  });

  it('throws when nothing rewriteable', () => {
    expect(() => parsePrefix('system="   "')).toThrow(FingerprintSampleParseError);
  });

  it('maps sample pools to sanitize tiers', () => {
    expect(sampleTier('trae')).toBe('wb');
    expect(sampleTier('buddy')).toBe('wb');
    expect(sampleTier('buddy-ai')).toBe('buddy-ai');
  });

  it('marks pools without a template rewrite path as diagnostic only', () => {
    // qoder 上游不落模板清洗，样本仅作诊断，不得生成规则
    expect(sampleTier('qoder')).toBeNull();
  });
});
