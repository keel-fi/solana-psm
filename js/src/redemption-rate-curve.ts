import {struct, u8} from '@solana/buffer-layout';
import {u128} from '@solana/buffer-layout-utils';
import {PublicKey, TransactionInstruction} from '@solana/web3.js';

export interface RawRedemptionRateCurve {
  maxSsr: bigint;
  ssr: bigint;
  rho: bigint;
  chi: bigint;
}

export interface SetRatesInstruction {
  instruction: number;
  ssr: bigint;
  rho: bigint;
  chi: bigint;
}

/** Struct for (de)serialization of RedemptionRateCurve */
export const RedemptionRateCurveLayout = struct<RawRedemptionRateCurve>([
  u128('maxSsr'),
  u128('ssr'),
  u128('rho'),
  u128('chi'),
]);

const SetRatesInstructionLayout = struct<SetRatesInstruction>([
  u8('instruction'),
  u128('ssr'),
  u128('rho'),
  u128('chi'),
]);

/**
 * Construct a SetRates instruction for the Solana PSM.
 * @param swapAccount
 * @param permissionAccount
 * @param authority
 * @param ssr
 * @param rho
 * @param chi
 * @returns
 */
export const getPsmSsrUpdateInstruction = (
  swapProgramId: PublicKey,
  swapAccount: PublicKey,
  permissionAccount: PublicKey,
  authority: PublicKey,
  ssr: bigint,
  rho: bigint,
  chi: bigint,
): TransactionInstruction => {
  // Encode instruction data to buffer
  const data = Buffer.alloc(SetRatesInstructionLayout.span);
  SetRatesInstructionLayout.encode(
    {
      instruction: 6,
      ssr,
      rho,
      chi,
    },
    data,
  );

  return new TransactionInstruction({
    keys: [
      {pubkey: swapAccount, isWritable: true, isSigner: false},
      {pubkey: permissionAccount, isWritable: false, isSigner: false},
      {pubkey: authority, isWritable: false, isSigner: true},
    ],
    programId: swapProgramId,
    data,
  });
};
