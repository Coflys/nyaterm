const LEADING_ENV_PREFIX = /^\(([^()\r\n]+)\)\s*/u;
const BRACKET_PROMPT_PREFIX = /^\[[^\]\r\n]+\]\s*[#$]\s*/u;
const POSIX_PROMPT_PREFIX = /^[^\s@]+@[^:\s]+:[^#$\r\n]*[#$]\s*/u;
const SIMPLE_POSIX_PROMPT_PREFIX = /^\$[ \t]+/u;
const POWERSHELL_PROMPT_PREFIX = /^PS\s+[^>\r\n]+>\s*/u;
const WINDOWS_PROMPT_PREFIX = /^[A-Za-z]:(?:[\\/][^>\r\n]*)?>\s*/u;

function stripLeadingEnvPrefixes(input: string): string {
  let remaining = input;

  while (true) {
    const match = remaining.match(LEADING_ENV_PREFIX);
    if (!match) {
      return remaining;
    }
    remaining = remaining.slice(match[0].length);
  }
}

function stripKnownPromptPrefix(input: string): string {
  for (const matcher of [
    BRACKET_PROMPT_PREFIX,
    POSIX_PROMPT_PREFIX,
    SIMPLE_POSIX_PROMPT_PREFIX,
    POWERSHELL_PROMPT_PREFIX,
    WINDOWS_PROMPT_PREFIX,
  ]) {
    const match = input.match(matcher);
    if (match) {
      return input.slice(match[0].length);
    }
  }

  return input;
}

/** Remove known shell prompt prefixes while preserving command spacing. */
export function stripTerminalCommandPrompt(input: string): string {
  const withoutLeadingWhitespace = input.trimStart();
  if (!withoutLeadingWhitespace.trim()) {
    return "";
  }

  return stripKnownPromptPrefix(stripLeadingEnvPrefixes(withoutLeadingWhitespace));
}

/** Remove known shell prompt prefixes so command parsing stays stable across shells. */
export function sanitizeTerminalCommand(input: string): string {
  const withoutPrompt = stripTerminalCommandPrompt(input);
  return withoutPrompt.trim();
}
