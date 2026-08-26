export interface ValidationResult {
  valid: boolean;
  reason?: string;
}

export function validateEmail(email: string): ValidationResult {
  const re = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;
  if (!re.test(email)) {
    return { valid: false, reason: "Email address is not valid" };
  }
  return { valid: true };
}

export function validatePassword(password: string): ValidationResult {
  if (password.length < 8) {
    return { valid: false, reason: "Password must be at least 8 characters" };
  }
  return { valid: true };
}

export function validateUsername(username: string): ValidationResult {
  if (username.length < 3 || username.length > 20) {
    return { valid: false, reason: "Username must be 3-20 characters" };
  }
  if (!/^[a-zA-Z0-9_]+$/.test(username)) {
    return {
      valid: false,
      reason: "Username may only contain letters, numbers, and underscores",
    };
  }
  return { valid: true };
}
