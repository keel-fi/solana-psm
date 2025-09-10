import {PublicKey} from '@solana/web3.js';

// SPDX-License-Identifier: AGPL-3.0-only
export * from './redemption-rate-curve.ts';
export * from './token-swap.ts';

export const SOLANA_PSM_PROGRAM_ID: PublicKey = new PublicKey(
  '5B9vCSSga3qXgHca5Liy3WAQqC2HaB3sBsyjfkH47uYv',
);

export const OLD_TOKEN_SWAP_PROGRAM_ID: PublicKey = new PublicKey(
  'SwaPpA9LAaLfeLi3a68M4DjnLqgtticKg6CnyNwgAC8',
);
