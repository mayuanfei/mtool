import { useState, useEffect, useMemo } from 'react';
import { Lock, Copy, Trash2, ArrowDown, Check, XCircle, Loader2 } from 'lucide-react';
import { useI18n } from '../i18n';
import CryptoJS from 'crypto-js';
import { sm2, sm3, sm4 } from 'sm-crypto';
import JSEncrypt from 'jsencrypt';
import { invoke } from '@tauri-apps/api/core';
import { CustomSelect } from '../components/CustomSelect';

type Category = 'hash' | 'symmetric' | 'asymmetric' | 'hq';
type Algorithm = 'MD5' | 'SHA1' | 'SHA256' | 'SM3' | 'AES' | 'DES' | '3DES' | 'SM4' | 'RSA' | 'SM2' | 'HQ_DLL';
type DataFormat = 'UTF8' | 'HEX' | 'BASE64';

function arrayBufferToBase64(buffer: ArrayBuffer): string {
  let binary = '';
  const bytes = new Uint8Array(buffer);
  const len = bytes.byteLength;
  for (let i = 0; i < len; i++) {
    binary += String.fromCharCode(bytes[i]);
  }
  return window.btoa(binary);
}

function formatAsPem(base64: string, type: 'PUBLIC' | 'PRIVATE'): string {
  const lines: string[] = [];
  lines.push(`-----BEGIN ${type} KEY-----`);
  for (let i = 0; i < base64.length; i += 64) {
    lines.push(base64.slice(i, i + 64));
  }
  lines.push(`-----END ${type} KEY-----`);
  return lines.join('\n');
}

async function generateRsaKeysWebCrypto(keySize: number): Promise<{ publicKey: string; privateKey: string }> {
  const keyPair = await window.crypto.subtle.generateKey(
    {
      name: "RSA-OAEP",
      modulusLength: keySize,
      publicExponent: new Uint8Array([1, 0, 1]), // 65537
      hash: "SHA-256",
    },
    true,
    ["encrypt", "decrypt"]
  );

  const spki = await window.crypto.subtle.exportKey("spki", keyPair.publicKey);
  const pkcs8 = await window.crypto.subtle.exportKey("pkcs8", keyPair.privateKey);

  const pubBase64 = arrayBufferToBase64(spki);
  const privBase64 = arrayBufferToBase64(pkcs8);

  return {
    publicKey: formatAsPem(pubBase64, 'PUBLIC'),
    privateKey: formatAsPem(privBase64, 'PRIVATE')
  };
}

export function CryptoTool() {
  const { t } = useI18n();
  const [category, setCategory] = useState<Category>('hash');
  const [algorithm, setAlgorithm] = useState<Algorithm>('MD5');

  const [input, setInput] = useState('');
  const [output, setOutput] = useState('');
  const [cryptoKey, setCryptoKey] = useState('');
  const [iv, setIv] = useState('');
  const [mode, setMode] = useState('CBC');
  const [padding, setPadding] = useState('Pkcs7');

  const [inputFormat, setInputFormat] = useState<DataFormat>('UTF8');
  const [outputFormat, setOutputFormat] = useState<DataFormat>('BASE64');
  const [keyFormat, setKeyFormat] = useState<DataFormat>('UTF8');
  const [ivFormat, setIvFormat] = useState<DataFormat>('UTF8');
  
  // For Asymmetric
  const [publicKey, setPublicKey] = useState('');
  const [privateKey, setPrivateKey] = useState('');
  const [rsaKeySize, setRsaKeySize] = useState('2048');
  const [sm2CipherMode, setSm2CipherMode] = useState('1'); // 1 = C1C3C2, 0 = C1C2C3

  // Active operation mode
  const [isEncrypt, setIsEncrypt] = useState(true);

  // For HQ DLL
  const [jarPath, setJarPath] = useState(() => localStorage.getItem('mtool_hq_jar') || '');
  const [bizType, setBizType] = useState(() => localStorage.getItem('mtool_hq_param') || 'A001');
  const [jdkPath, setJdkPath] = useState(() => localStorage.getItem('mtool_hq_jdk') || '');

  useEffect(() => {
    localStorage.setItem('mtool_hq_jar', jarPath);
  }, [jarPath]);

  useEffect(() => {
    localStorage.setItem('mtool_hq_param', bizType);
  }, [bizType]);

  useEffect(() => {
    localStorage.setItem('mtool_hq_jdk', jdkPath);
  }, [jdkPath]);

  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [copyError, setCopyError] = useState(false);
  const [isLoading, setIsLoading] = useState(false);

  const categories: { value: Category; label: string }[] = [
    { value: 'hash', label: t('Hash / Digest') },
    { value: 'symmetric', label: t('Symmetric') },
    { value: 'asymmetric', label: t('Asymmetric') },
    { value: 'hq', label: t('HQ DLL') },
  ];

  const algorithms: Record<Category, Algorithm[]> = {
    hash: ['MD5', 'SHA1', 'SHA256', 'SM3'],
    symmetric: ['AES', 'DES', '3DES', 'SM4'],
    asymmetric: ['RSA', 'SM2'],
    hq: ['HQ_DLL'],
  };

  const handleCategoryChange = (c: Category) => {
    setCategory(c);
    setAlgorithm(algorithms[c][0]);
    setOutput('');
    setError(null);
    if (c === 'hash') {
      setIsEncrypt(true);
      setOutputFormat('HEX');
    } else {
      setIsEncrypt(true);
      setOutputFormat('BASE64');
      setInputFormat('UTF8');
    }
  };

  const handleAlgorithmChange = (a: Algorithm) => {
    setAlgorithm(a);
    setOutput('');
    setError(null);
  };

  const getCryptoJsMode = () => {
    return mode === 'ECB' ? CryptoJS.mode.ECB : CryptoJS.mode.CBC;
  };

  const getCryptoJsPadding = () => {
    if (padding === 'ZeroPadding') return CryptoJS.pad.ZeroPadding;
    if (padding === 'NoPadding') return CryptoJS.pad.NoPadding;
    return CryptoJS.pad.Pkcs7;
  };

  const parseData = (data: string, format: DataFormat) => {
    if (format === 'HEX') return CryptoJS.enc.Hex.parse(data);
    if (format === 'BASE64') return CryptoJS.enc.Base64.parse(data);
    return CryptoJS.enc.Utf8.parse(data);
  };

  const stringifyData = (wordArray: CryptoJS.lib.WordArray | undefined, format: DataFormat) => {
    if (!wordArray) return '';
    if (format === 'HEX') return CryptoJS.enc.Hex.stringify(wordArray);
    if (format === 'BASE64') return CryptoJS.enc.Base64.stringify(wordArray);
    return CryptoJS.enc.Utf8.stringify(wordArray);
  };

  const toHex = (data: string, format: DataFormat) => {
    return CryptoJS.enc.Hex.stringify(parseData(data, format));
  };
  const fromHex = (hexData: string, format: DataFormat) => {
    return stringifyData(CryptoJS.enc.Hex.parse(hexData), format);
  };

  const forceSize = (wordArray: CryptoJS.lib.WordArray, targetBytes: number) => {
    const words = wordArray.words;
    
    const newWords = [];
    for (let i = 0; i < Math.ceil(targetBytes / 4); i++) {
      newWords.push(words[i] || 0);
    }
    
    const lastWordIndex = Math.ceil(targetBytes / 4) - 1;
    const remainingBytes = targetBytes % 4;
    if (remainingBytes !== 0 && newWords[lastWordIndex] !== undefined) {
      const mask = 0xffffffff << (8 * (4 - remainingBytes));
      newWords[lastWordIndex] &= mask;
    }
    
    return CryptoJS.lib.WordArray.create(newWords, targetBytes);
  };

  const adjustKeySize = (keyWa: CryptoJS.lib.WordArray, algo: string) => {
    if (algo === 'AES') {
      const bytes = keyWa.sigBytes;
      if (bytes <= 16) return forceSize(keyWa, 16);
      if (bytes <= 24) return forceSize(keyWa, 24);
      return forceSize(keyWa, 32);
    }
    if (algo === 'DES') {
      return forceSize(keyWa, 8);
    }
    if (algo === '3DES') {
      const bytes = keyWa.sigBytes;
      if (bytes === 16) {
        // Copy first 8 bytes to the end (K3 = K1)
        const words = [...keyWa.words];
        words[4] = words[0];
        words[5] = words[1];
        return CryptoJS.lib.WordArray.create(words, 24);
      }
      return forceSize(keyWa, 24);
    }
    if (algo === 'SM4') {
      return forceSize(keyWa, 16);
    }
    return keyWa;
  };

  const adjustIvSize = (ivWa: CryptoJS.lib.WordArray, algo: string) => {
    if (algo === 'AES') {
      return forceSize(ivWa, 16);
    }
    if (algo === 'DES' || algo === '3DES') {
      return forceSize(ivWa, 8);
    }
    if (algo === 'SM4') {
      return forceSize(ivWa, 16);
    }
    return ivWa;
  };

  const { keyWarning, ivWarning } = useMemo(() => {
    if (category !== 'symmetric') return { keyWarning: null, ivWarning: null };
    if (!cryptoKey) return { keyWarning: null, ivWarning: null };

    let keyWarning: string | null = null;
    let ivWarning: string | null = null;

    try {
      const parsedKey = parseData(cryptoKey, keyFormat);
      const keyBytes = parsedKey.sigBytes;
      let targetKeyBytes = 0;

      if (algorithm === 'AES') {
        if (keyBytes > 0 && keyBytes !== 16 && keyBytes !== 24 && keyBytes !== 32) {
          targetKeyBytes = keyBytes <= 16 ? 16 : keyBytes <= 24 ? 24 : 32;
        }
      } else if (algorithm === 'DES') {
        if (keyBytes > 0 && keyBytes !== 8) {
          targetKeyBytes = 8;
        }
      } else if (algorithm === '3DES') {
        if (keyBytes > 0 && keyBytes !== 24 && keyBytes !== 16) {
          targetKeyBytes = 24;
        }
      } else if (algorithm === 'SM4') {
        if (keyBytes > 0 && keyBytes !== 16) {
          targetKeyBytes = 16;
        }
      }

      if (targetKeyBytes > 0) {
        const actionStr = keyBytes < targetKeyBytes ? t('padded') : t('truncated');
        keyWarning = t('Key length mismatch (current: {current}B), it will be {action} to {target}B.', {
          current: keyBytes,
          action: actionStr,
          target: targetKeyBytes
        });
      }
    } catch (e) {
      keyWarning = t('Invalid key format');
    }

    if (mode === 'CBC' && iv) {
      try {
        const parsedIv = parseData(iv, ivFormat);
        const ivBytes = parsedIv.sigBytes;
        let targetIvBytes = 0;

        if (algorithm === 'AES' || algorithm === 'SM4') {
          if (ivBytes > 0 && ivBytes !== 16) {
            targetIvBytes = 16;
          }
        } else if (algorithm === 'DES' || algorithm === '3DES') {
          if (ivBytes > 0 && ivBytes !== 8) {
            targetIvBytes = 8;
          }
        }

        if (targetIvBytes > 0) {
          const actionStr = ivBytes < targetIvBytes ? t('padded') : t('truncated');
          ivWarning = t('IV length mismatch (current: {current}B), it will be {action} to {target}B.', {
            current: ivBytes,
            action: actionStr,
            target: targetIvBytes
          });
        }
      } catch (e) {
        ivWarning = t('Invalid IV format');
      }
    }

    return { keyWarning, ivWarning };
  }, [category, cryptoKey, keyFormat, algorithm, mode, iv, ivFormat, t]);

  const handleAction = async (doEncrypt: boolean) => {
    if (!input) {
      setError(t('Payload is empty'));
      return;
    }
    if (!doEncrypt && category !== 'hq' && inputFormat === 'UTF8') {
      setError(t('Decryption requires binary ciphertext. Please select HEX or BASE64 as the input format.'));
      return;
    }
    if (isLoading) return;
    setIsLoading(true);
    setError(null);
    
    // Provide visual feedback for recalculation by temporarily clearing the output
    if (output) {
      setOutput('');
      await new Promise((resolve) => setTimeout(resolve, 50));
    }

    let currentOutputFormat = outputFormat;
    if (doEncrypt && outputFormat === 'UTF8') {
      currentOutputFormat = 'BASE64';
    }

    try {
      let res = '';
      
      // ================= HASH =================
      if (category === 'hash') {
        if (!doEncrypt) throw new Error(t('Hash algorithms cannot be decrypted.'));
        const inputWa = parseData(input, inputFormat);
        let hashWa;
        if (algorithm === 'MD5') hashWa = CryptoJS.MD5(inputWa);
        else if (algorithm === 'SHA1') hashWa = CryptoJS.SHA1(inputWa);
        else if (algorithm === 'SHA256') hashWa = CryptoJS.SHA256(inputWa);
        else if (algorithm === 'SM3') {
          const hexIn = toHex(input, inputFormat);
          const bytes = [];
          for (let c = 0; c < hexIn.length; c += 2) {
            bytes.push(parseInt(hexIn.slice(c, c + 2), 16));
          }
          hashWa = CryptoJS.enc.Hex.parse(sm3(bytes));
        }
        res = (currentOutputFormat === 'UTF8' && hashWa)
          ? CryptoJS.enc.Base64.stringify(hashWa)
          : stringifyData(hashWa, currentOutputFormat);
      } 
      // ================= SYMMETRIC =================
      else if (category === 'symmetric') {
        if (!cryptoKey) throw new Error(t('Key is required.'));
        
        if (algorithm === 'AES' || algorithm === 'DES' || algorithm === '3DES') {
          const cfg: {
            mode: typeof CryptoJS.mode.CBC;
            padding: typeof CryptoJS.pad.Pkcs7;
            iv?: CryptoJS.lib.WordArray;
          } = { mode: getCryptoJsMode(), padding: getCryptoJsPadding() };
          if (mode === 'CBC') {
            if (!iv) throw new Error(t('IV is required for CBC mode.'));
            cfg.iv = adjustIvSize(parseData(iv, ivFormat), algorithm);
          }
          const keyWa = adjustKeySize(parseData(cryptoKey, keyFormat), algorithm);
          const inputWa = parseData(input, inputFormat);
          
          let engine = algorithm === 'AES' ? CryptoJS.AES : algorithm === 'DES' ? CryptoJS.DES : CryptoJS.TripleDES;

          if (doEncrypt) {
            const encrypted = engine.encrypt(inputWa, keyWa, cfg);
            res = currentOutputFormat === 'HEX'
              ? encrypted.ciphertext.toString(CryptoJS.enc.Hex)
              : encrypted.toString();
          } else {
            // For decryption, CryptoJS takes Base64 string or CipherParams
            let decryptInput = input.trim();
            if (inputFormat === 'HEX') {
              decryptInput = CryptoJS.enc.Base64.stringify(CryptoJS.enc.Hex.parse(decryptInput));
            }
            const decrypted = engine.decrypt(decryptInput, keyWa, cfg);
            res = stringifyData(decrypted, currentOutputFormat);
            if (!res) throw new Error(t('Decryption failed. Invalid Key/IV or data.'));
          }
        } 
        else if (algorithm === 'SM4') {
          const rawKeyWa = parseData(cryptoKey, keyFormat);
          const keyHex = CryptoJS.enc.Hex.stringify(adjustKeySize(rawKeyWa, 'SM4'));
          const cfg: Record<string, string> = {};
          if (mode === 'CBC') {
            if (!iv) throw new Error(t('IV is required for CBC mode.'));
            const rawIvWa = parseData(iv, ivFormat);
            cfg.iv = CryptoJS.enc.Hex.stringify(adjustIvSize(rawIvWa, 'SM4'));
            cfg.mode = 'cbc';
          }
          
          let inputHex = '';
          if (doEncrypt) {
            inputHex = toHex(input, inputFormat);
          } else {
            const cipherText = input.trim();
            if (inputFormat === 'BASE64' || inputFormat === 'UTF8') {
              inputHex = CryptoJS.enc.Hex.stringify(CryptoJS.enc.Base64.parse(cipherText));
            } else if (inputFormat === 'HEX') {
              inputHex = cipherText;
            }
          }
          // convert hex to byte array for sm-crypto
          const hexToBytes = (hex: string) => {
            let bytes = [];
            for (let c = 0; c < hex.length; c += 2) bytes.push(parseInt(hex.slice(c, c + 2), 16));
            return bytes;
          };

          if (doEncrypt) {
            let inBytes = hexToBytes(inputHex);
            if (padding === 'ZeroPadding') {
              cfg.padding = 'none';
              const paddingCount = 16 - (inBytes.length % 16);
              for (let i = 0; i < paddingCount; i++) inBytes.push(0);
            } else if (padding === 'NoPadding') {
              cfg.padding = 'none';
              if (inBytes.length % 16 !== 0) throw new Error(t('Data length must be a multiple of 16 bytes for NoPadding.'));
            } else {
              cfg.padding = 'pkcs#7';
            }
            const outHex = sm4.encrypt(inBytes, keyHex, cfg);
            res = fromHex(outHex, currentOutputFormat);
          } else {
            if (padding === 'ZeroPadding' || padding === 'NoPadding') {
              cfg.padding = 'none';
            } else {
              cfg.padding = 'pkcs#7';
            }
            const outBytes = sm4.decrypt(hexToBytes(inputHex), keyHex, { ...cfg, output: 'array' }) as unknown as number[];
            if (!outBytes) throw new Error(t('Decryption failed. Invalid Key/IV or data.'));
            
            let unpaddedBytes = outBytes;
            if (padding === 'ZeroPadding') {
              let len = unpaddedBytes.length;
              while (len > 0 && unpaddedBytes[len - 1] === 0) len--;
              unpaddedBytes = unpaddedBytes.slice(0, len);
            }
            const outHex = unpaddedBytes.map((b: number) => b.toString(16).padStart(2, '0')).join('');
            res = fromHex(outHex, currentOutputFormat);
          }
        }
      }
      // ================= ASYMMETRIC =================
      else if (category === 'asymmetric') {
        if (algorithm === 'RSA') {
          res = await new Promise<string>((resolve, reject) => {
            setTimeout(() => {
              try {
                const encryptor = new JSEncrypt();
                if (doEncrypt) {
                  if (!publicKey) return reject(new Error(t('Public Key is required for encryption.')));
                  encryptor.setPublicKey(publicKey);
                  
                  let plaintext = input;
                  if (inputFormat === 'HEX') {
                    try {
                      plaintext = CryptoJS.enc.Utf8.stringify(CryptoJS.enc.Hex.parse(input));
                    } catch (e) {
                      throw new Error(t('Invalid HEX/BASE64 input'));
                    }
                  } else if (inputFormat === 'BASE64') {
                    try {
                      plaintext = CryptoJS.enc.Utf8.stringify(CryptoJS.enc.Base64.parse(input));
                    } catch (e) {
                      throw new Error(t('Invalid HEX/BASE64 input'));
                    }
                  }

                  // 尺寸预检：根据 modulus 位数或配置的 keySize 计算最大允许字节数
                  // JSEncrypt 默认使用 PKCS#1 v1.5 填充，最大可加密数据长度为：模长(字节数) - 11
                  let bitLen = parseInt(rsaKeySize, 10);
                  const keyObj = encryptor.getKey();
                  const n = (keyObj as any).n;
                  if (n) {
                    bitLen = n.bitLength();
                  }
                  const maxBytes = Math.floor(bitLen / 8) - 11;
                  const byteLen = new TextEncoder().encode(plaintext).length;
                  if (byteLen > maxBytes) {
                    return reject(new Error(t('RSA plaintext is too long ({len} bytes). For {size}-bit key, the maximum is {max} bytes.', {
                      len: byteLen,
                      size: bitLen,
                      max: maxBytes
                    })));
                  }

                  const encoded = encryptor.encrypt(plaintext);
                  if (!encoded) return reject(new Error(t('RSA Encryption failed.')));
                  
                  let encryptRes = '';
                  if (currentOutputFormat === 'HEX') {
                    encryptRes = CryptoJS.enc.Hex.stringify(CryptoJS.enc.Base64.parse(encoded));
                  } else {
                    encryptRes = encoded; // Default is BASE64
                  }
                  resolve(encryptRes);
                } else {
                  if (!privateKey) return reject(new Error(t('Private Key is required for decryption.')));
                  encryptor.setPrivateKey(privateKey);
                  
                  let ciphertextBase64 = input.trim();
                  if (inputFormat === 'HEX') {
                    ciphertextBase64 = CryptoJS.enc.Base64.stringify(CryptoJS.enc.Hex.parse(ciphertextBase64));
                  }
                  
                  const decoded = encryptor.decrypt(ciphertextBase64);
                  if (!decoded) return reject(new Error(t('RSA Decryption failed.')));
                  
                  let decryptRes = '';
                  if (currentOutputFormat === 'HEX') {
                    decryptRes = CryptoJS.enc.Hex.stringify(CryptoJS.enc.Utf8.parse(decoded));
                  } else if (currentOutputFormat === 'BASE64') {
                    decryptRes = CryptoJS.enc.Base64.stringify(CryptoJS.enc.Utf8.parse(decoded));
                  } else {
                    decryptRes = decoded; // Default UTF-8
                  }
                  resolve(decryptRes);
                }
              } catch (e) {
                reject(e);
              }
            }, 50);
          });
        }
        else if (algorithm === 'SM2') {
          if (doEncrypt) {
            if (!publicKey) throw new Error(t('Public Key is required for encryption.'));
            
            let plaintext = input;
            if (inputFormat === 'HEX') {
              try {
                plaintext = CryptoJS.enc.Utf8.stringify(CryptoJS.enc.Hex.parse(input));
              } catch (e) {
                throw new Error(t('Invalid HEX/BASE64 input'));
              }
            } else if (inputFormat === 'BASE64') {
              try {
                plaintext = CryptoJS.enc.Utf8.stringify(CryptoJS.enc.Base64.parse(input));
              } catch (e) {
                throw new Error(t('Invalid HEX/BASE64 input'));
              }
            }
            
            const cipherHex = sm2.doEncrypt(plaintext, publicKey, parseInt(sm2CipherMode, 10) as 0 | 1);
            if (!cipherHex || cipherHex === 'null') throw new Error(t('SM2 Encryption failed.'));
            
            if (currentOutputFormat === 'BASE64') {
              res = CryptoJS.enc.Base64.stringify(CryptoJS.enc.Hex.parse(cipherHex));
            } else {
              res = cipherHex; // Default is HEX
            }
          } else {
            if (!privateKey) throw new Error(t('Private Key is required for decryption.'));
            
            let ciphertextHex = input.trim();
            if (inputFormat === 'BASE64' || inputFormat === 'UTF8') {
              ciphertextHex = CryptoJS.enc.Hex.stringify(CryptoJS.enc.Base64.parse(ciphertextHex));
            }
            
            const decoded = sm2.doDecrypt(ciphertextHex, privateKey, parseInt(sm2CipherMode, 10) as 0 | 1, { output: 'string' });
            if (!decoded) throw new Error(t('SM2 Decryption failed.'));
            
            if (currentOutputFormat === 'HEX') {
              res = CryptoJS.enc.Hex.stringify(CryptoJS.enc.Utf8.parse(decoded));
            } else if (currentOutputFormat === 'BASE64') {
              res = CryptoJS.enc.Base64.stringify(CryptoJS.enc.Utf8.parse(decoded));
            } else {
              res = decoded; // Default UTF-8
            }
          }
        }
      }
      // ================= HQ DLL =================
      else if (category === 'hq') {
        // Call Rust Backend Command
        try {
          if (!jarPath) throw new Error(t('Please select the HQ Jar file.'));
          res = await invoke('hq_crypto', { 
            action: doEncrypt ? 'enc' : 'dec',
            payload: input,
            jarPath,
            bizType,
            jdkPath: jdkPath || null
          });
        } catch (err: unknown) {
          throw new Error(`HQ DLL Error: ${err instanceof Error ? err.message : String(err)}`);
        }
      }

      if (res === 'null' || res === null || res === undefined) {
        setOutput('');
      } else {
        setOutput(res);
      }
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setIsLoading(false);
    }
  };

  const copyOutput = async () => {
    if (!output) return;
    try {
      await navigator.clipboard.writeText(output);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch (err) {
      console.error('Failed to copy', err);
      setCopyError(true);
      setTimeout(() => setCopyError(false), 2000);
    }
  };

  const generateKeys = async () => {
    if (isLoading) return;
    setError(null);
    setIsLoading(true);
    try {
      if (algorithm === 'RSA') {
        const size = parseInt(rsaKeySize, 10);
        const keys = await generateRsaKeysWebCrypto(size);
        setPublicKey(keys.publicKey);
        setPrivateKey(keys.privateKey);
      } else if (algorithm === 'SM2') {
        const keypair = sm2.generateKeyPairHex();
        setPublicKey(keypair.publicKey);
        setPrivateKey(keypair.privateKey);
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setIsLoading(false);
    }
  };

  return (
    <div className="w-full h-full flex flex-col min-h-0">
      <div className="flex justify-between items-center mb-6 border-b th-border pb-4 shrink-0">
        <h2 className="th-text font-semibold text-lg flex items-center gap-2">
          <Lock className="w-5 h-5 text-indigo-400" />
          {t('Crypto Tool')}
        </h2>
      </div>

      <div className="flex-1 flex flex-col gap-4 min-h-0">
        {/* Top Navigation */}
        <div className="flex flex-wrap gap-2 flex-shrink-0 bg-indigo-500/5 p-1 rounded-lg border th-border">
          {categories.map((c) => (
            <button
              key={c.value}
              onClick={() => handleCategoryChange(c.value)}
              className={`px-4 py-2 rounded-md text-sm font-medium transition-colors ${
                category === c.value 
                  ? 'bg-indigo-600 text-white shadow-sm' 
                  : 'th-text-2 hover:bg-indigo-500/10'
              }`}
            >
              {c.label}
            </button>
          ))}
        </div>

        {/* Algorithm Selector */}
        {category !== 'hq' && (
          <div className="flex flex-wrap gap-2 flex-shrink-0">
            {algorithms[category].map((a) => (
              <button
                key={a}
                onClick={() => handleAlgorithmChange(a)}
                className={`px-3 py-1.5 rounded-md text-xs font-medium transition-colors border ${
                  algorithm === a 
                    ? 'bg-indigo-500/20 text-indigo-400 border-indigo-500/30' 
                    : 'th-bg-surface th-text-3 border-transparent hover:border-indigo-500/20'
                }`}
              >
                {a}
              </button>
            ))}
          </div>
        )}



        {/* Configuration Area */}
        {category === 'symmetric' && (
          <div className="grid grid-cols-4 gap-4 shrink-0 border th-border rounded-xl p-4 th-bg-surface-h">
            <div className="flex flex-col gap-1.5 col-span-1">
              <label className="text-xs font-semibold th-text-muted uppercase">{t('Mode')}</label>
              <CustomSelect 
                value={mode}
                onChange={(val: string) => setMode(val)}
                options={[{ value: 'CBC', label: 'CBC' }, { value: 'ECB', label: 'ECB' }]}
                className="w-full px-3 py-2 border th-border rounded-lg th-text text-sm focus:ring-1 focus:ring-indigo-500"
              />
            </div>
            <div className="flex flex-col gap-1.5 col-span-1">
              <label className="text-xs font-semibold th-text-muted uppercase">{t('Padding')}</label>
              <CustomSelect 
                value={padding}
                onChange={(val: string) => setPadding(val)}
                options={[{ value: 'Pkcs7', label: 'PKCS7' }, { value: 'ZeroPadding', label: 'ZeroPadding' }, { value: 'NoPadding', label: 'NoPadding' }]}
                className="w-full px-3 py-2 border th-border rounded-lg th-text text-sm focus:ring-1 focus:ring-indigo-500"
              />
            </div>
            <div className="flex flex-col gap-1.5 col-span-1">
              <div className="flex justify-between items-center">
                <label className="text-xs font-semibold th-text-muted uppercase">{t('Key')}</label>
                <CustomSelect 
                  value={keyFormat}
                  onChange={(val: DataFormat) => setKeyFormat(val)}
                  options={[{ value: 'UTF8', label: t('UTF8') }, { value: 'HEX', label: t('HEX') }, { value: 'BASE64', label: t('BASE64') }]}
                  className="text-[10px] th-text-3 hover:text-indigo-400 font-medium"
                  menuClassName="right-0 min-w-[6rem]"
                />
              </div>
              <input 
                type="text"
                value={cryptoKey}
                onChange={(e) => setCryptoKey(e.target.value)}
                placeholder="Secret Key"
                className="w-full px-3 py-2 bg-transparent border th-border rounded-lg th-text text-sm focus:ring-1 focus:ring-indigo-500 outline-none placeholder:text-gray-500/40"
              />
              {keyWarning && (
                <span className="text-[10px] text-amber-500 font-medium leading-normal flex items-start gap-1">
                  <span>⚠️</span>
                  <span>{keyWarning}</span>
                </span>
              )}
            </div>
            <div className="flex flex-col gap-1.5 col-span-1">
              <div className="flex justify-between items-center">
                <label className="text-xs font-semibold th-text-muted uppercase">{t('IV')}</label>
                <CustomSelect 
                  value={ivFormat}
                  onChange={(val: DataFormat) => setIvFormat(val)}
                  disabled={mode === 'ECB'}
                  options={[{ value: 'UTF8', label: t('UTF8') }, { value: 'HEX', label: t('HEX') }, { value: 'BASE64', label: t('BASE64') }]}
                  className="text-[10px] th-text-3 hover:text-indigo-400 font-medium"
                  menuClassName="right-0 min-w-[6rem]"
                />
              </div>
              <input 
                type="text"
                value={iv}
                onChange={(e) => setIv(e.target.value)}
                disabled={mode === 'ECB'}
                placeholder={mode === 'ECB' ? t('Not needed for ECB') : 'Initialization Vector'}
                className="w-full px-3 py-2 bg-transparent border th-border rounded-lg th-text text-sm focus:ring-1 focus:ring-indigo-500 outline-none disabled:opacity-50 placeholder:text-gray-500/40"
              />
              {ivWarning && (
                <span className="text-[10px] text-amber-500 font-medium leading-normal flex items-start gap-1">
                  <span>⚠️</span>
                  <span>{ivWarning}</span>
                </span>
              )}
            </div>
          </div>
        )}

        {category === 'asymmetric' && (
          <div className="flex flex-col gap-4 shrink-0 border th-border rounded-xl p-4 th-bg-surface-h">
            <div className="flex justify-between items-center">
              <span className="text-xs font-semibold th-text-muted uppercase">{t('Keys Configuration')}</span>
              <div className="flex items-center gap-3">
                {algorithm === 'RSA' && (
                  <div className="flex items-center gap-1.5">
                    <span className="text-[10px] th-text-3 font-medium">{t('Key Size')}:</span>
                    <CustomSelect 
                      value={rsaKeySize}
                      onChange={(val: string) => setRsaKeySize(val)}
                      options={[{ value: '1024', label: '1024' }, { value: '2048', label: '2048' }, { value: '4096', label: '4096' }]}
                      className="text-[10px] th-text-3 hover:text-indigo-400 font-medium"
                      menuClassName="right-0 min-w-[6rem]"
                    />
                  </div>
                )}
                {algorithm === 'SM2' && (
                  <div className="flex items-center gap-1.5">
                    <span className="text-[10px] th-text-3 font-medium">{t('Cipher Mode')}:</span>
                    <CustomSelect 
                      value={sm2CipherMode}
                      onChange={(val: string) => setSm2CipherMode(val)}
                      options={[{ value: '1', label: 'C1C3C2 (New)' }, { value: '0', label: 'C1C2C3 (Old)' }]}
                      className="text-[10px] th-text-3 hover:text-indigo-400 font-medium"
                      menuClassName="right-0 min-w-[8rem]"
                    />
                  </div>
                )}
                <button 
                  onClick={generateKeys}
                  disabled={isLoading}
                  className={`text-xs font-medium transition-colors ${isLoading ? 'text-indigo-400/50 cursor-not-allowed' : 'text-indigo-400 hover:text-indigo-300'}`}
                >
                  {isLoading ? t('Generating...') : t('Auto Generate Key Pair')}
                </button>
              </div>
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div className="flex flex-col gap-1.5 col-span-1">
                <label className="text-xs th-text-3">{t('Public Key')}</label>
                <textarea 
                  value={publicKey}
                  onChange={(e) => setPublicKey(e.target.value)}
                  placeholder="-----BEGIN PUBLIC KEY-----..."
                  className="w-full h-24 p-3 bg-transparent border th-border rounded-lg th-text text-xs font-mono resize-none focus:ring-1 focus:ring-indigo-500 outline-none placeholder:text-gray-500/40"
                />
              </div>
              <div className="flex flex-col gap-1.5 col-span-1">
                <label className="text-xs th-text-3">{t('Private Key')}</label>
                <textarea 
                  value={privateKey}
                  onChange={(e) => setPrivateKey(e.target.value)}
                  placeholder="-----BEGIN PRIVATE KEY-----..."
                  className="w-full h-24 p-3 bg-transparent border th-border rounded-lg th-text text-xs font-mono resize-none focus:ring-1 focus:ring-indigo-500 outline-none placeholder:text-gray-500/40"
                />
              </div>
            </div>
          </div>
        )}

        {category === 'hq' && (
          <div className="flex flex-col gap-3 shrink-0 border border-indigo-500/20 bg-indigo-500/5 rounded-xl p-4 text-sm text-indigo-400">
            <div className="flex items-center gap-2">
              <span className="font-semibold whitespace-nowrap">{t('Jar Path')}</span>
              <div className="flex items-center gap-2 flex-1">
                <input 
                  type="text"
                  value={jarPath}
                  readOnly
                  placeholder={t('Select the ums-sm4.jar file (Natives directory must be present)')}
                  className="flex-1 bg-transparent border-b border-indigo-500/30 px-2 py-1 outline-none text-xs text-indigo-300 min-w-0 truncate"
                />
                <button 
                  onClick={async () => {
                    try {
                      const path = await invoke('select_hq_jar');
                      setJarPath(path as string);
                    } catch (e) {}
                  }}
                  className="px-3 py-1 bg-indigo-500/20 rounded hover:bg-indigo-500/30 transition-colors text-xs cursor-pointer whitespace-nowrap"
                >
                  {t('Browse')}
                </button>
              </div>
            </div>
            <div className="flex items-center gap-2">
              <span className="font-semibold whitespace-nowrap">{t('JDK Path')}</span>
              <div className="flex items-center gap-2 flex-1">
                <input 
                  type="text"
                  value={jdkPath}
                  readOnly
                  placeholder={t('Optional: select JDK/JRE directory (default: system PATH)')}
                  className="flex-1 bg-transparent border-b border-indigo-500/30 px-2 py-1 outline-none text-xs text-indigo-300 min-w-0 truncate"
                />
                <button 
                  onClick={async () => {
                    try {
                      const path = await invoke('select_jdk_dir');
                      setJdkPath(path as string);
                    } catch (e) {}
                  }}
                  className="px-3 py-1 bg-indigo-500/20 rounded hover:bg-indigo-500/30 transition-colors text-xs cursor-pointer whitespace-nowrap"
                >
                  {t('Browse')}
                </button>
                {jdkPath && (
                  <button 
                    onClick={() => setJdkPath('')}
                    className="px-2 py-1 bg-rose-500/20 rounded hover:bg-rose-500/30 transition-colors text-xs cursor-pointer text-rose-400 whitespace-nowrap"
                    title={t('Clear')}
                  >
                    ✕
                  </button>
                )}
              </div>
            </div>
            <div className="flex items-center gap-2">
              <span className="font-semibold whitespace-nowrap">{t('Custom Parameter')}</span>
              <input 
                type="text"
                value={bizType}
                onChange={(e) => setBizType(e.target.value)}
                placeholder="A001"
                className="w-48 bg-indigo-500/10 border-b border-indigo-500/30 px-3 py-1 outline-none text-xs text-indigo-300 transition-colors focus:bg-indigo-500/20"
              />
            </div>
          </div>
        )}

        <div className="flex-1 flex gap-4 min-h-0">
          
          {/* Input Panel */}
          <div className="flex-1 flex flex-col min-h-0 border th-border rounded-xl overflow-hidden shadow-sm th-bg-card">
            <div className="px-4 py-3 border-b th-border th-bg-surface-h flex items-center justify-between">
              <div className="flex items-center gap-3">
                <span className="font-semibold text-sm th-text-2 uppercase tracking-tight">{t('Input')}</span>
                {category !== 'hq' && (
                  <CustomSelect 
                    value={inputFormat}
                    onChange={(val: DataFormat) => setInputFormat(val)}
                    options={
                      (!isEncrypt && category !== 'hash')
                        ? [{ value: 'HEX', label: t('HEX') }, { value: 'BASE64', label: t('BASE64') }]
                        : [{ value: 'UTF8', label: t('UTF8') }, { value: 'HEX', label: t('HEX') }, { value: 'BASE64', label: t('BASE64') }]
                    }
                    className="text-xs th-text-3 hover:text-indigo-400 font-medium bg-indigo-500/5 px-2 py-1 rounded"
                    menuClassName="left-0 min-w-[6rem]"
                  />
                )}
              </div>
              <button 
                onClick={() => { setInput(''); setOutput(''); setError(null); }}
                className="p-1.5 th-text-muted hover:text-rose-400 hover:bg-rose-500/10 rounded transition-colors"
                title={t('Clear Input')}
              >
                <Trash2 className="w-4 h-4" />
              </button>
            </div>
            <textarea
              value={input}
              onChange={(e) => setInput(e.target.value)}
              className="flex-1 w-full p-4 bg-transparent border-none focus:ring-0 th-text placeholder:text-gray-500/40 resize-none font-mono text-sm leading-relaxed"
              placeholder="..."
              spellCheck={false}
            />
          </div>

          {/* Action Center */}
          <div className="flex flex-col justify-center gap-4 shrink-0 px-2">
            {category === 'hash' ? (
              <button
                onClick={() => handleAction(true)}
                disabled={isLoading}
                className={`flex flex-col items-center justify-center gap-1 min-w-[6rem] px-3 py-3 bg-indigo-600 hover:bg-indigo-700 text-white rounded-xl shadow-lg transition-all text-center ${isLoading ? 'opacity-50 cursor-not-allowed' : 'active:scale-95'}`}
              >
                <span className="font-bold text-sm">
                  {t('Hash')}
                </span>
                {isLoading ? (
                  <Loader2 className="w-4 h-4 animate-spin" />
                ) : (
                  <ArrowDown className="w-4 h-4 -rotate-90" />
                )}
              </button>
            ) : (
              <>
                <button
                  onClick={() => {
                    setIsEncrypt(true);
                    if (outputFormat === 'UTF8') {
                      setOutputFormat('BASE64');
                    }
                    handleAction(true);
                  }}
                  disabled={isLoading}
                  className={`flex flex-col items-center justify-center gap-1 min-w-[6rem] px-3 py-3 bg-indigo-600 hover:bg-indigo-700 text-white rounded-xl shadow-lg transition-all text-center ${isLoading ? 'opacity-50 cursor-not-allowed' : 'active:scale-95'}`}
                >
                  <span className="font-bold text-sm">
                    {category === 'asymmetric' ? t('Public Key Encrypt') : t('Encrypt')}
                  </span>
                  {isLoading && isEncrypt ? (
                    <Loader2 className="w-4 h-4 animate-spin" />
                  ) : (
                    <ArrowDown className="w-4 h-4 -rotate-90" />
                  )}
                </button>
                <button
                  onClick={() => {
                    setIsEncrypt(false);
                    if (inputFormat === 'UTF8') {
                      setInputFormat('BASE64');
                    }
                    handleAction(false);
                  }}
                  disabled={isLoading}
                  className={`flex flex-col items-center justify-center gap-1 min-w-[6rem] px-3 py-3 bg-indigo-600 hover:bg-indigo-700 text-white rounded-xl shadow-lg transition-all text-center ${isLoading ? 'opacity-50 cursor-not-allowed' : 'active:scale-95'}`}
                >
                  <span className="font-bold text-sm">
                    {category === 'asymmetric' ? t('Private Key Decrypt') : t('Decrypt')}
                  </span>
                  {isLoading && !isEncrypt ? (
                    <Loader2 className="w-4 h-4 animate-spin" />
                  ) : (
                    <ArrowDown className="w-4 h-4 -rotate-90" />
                  )}
                </button>
              </>
            )}
          </div>

          {/* Output Panel */}
          <div className="flex-1 flex flex-col min-h-0 border th-border rounded-xl overflow-hidden shadow-sm th-bg-card">
            <div className="px-4 py-3 border-b th-border th-bg-surface-h flex items-center justify-between">
              <div className="flex items-center gap-3">
                <span className="font-semibold text-sm th-text-2 uppercase tracking-tight">{t('Output')}</span>
                {category !== 'hq' && (
                  <CustomSelect 
                    value={outputFormat}
                    onChange={(val: DataFormat) => setOutputFormat(val)}
                    options={
                      (category === 'hash' || isEncrypt)
                        ? [{ value: 'HEX', label: t('HEX') }, { value: 'BASE64', label: t('BASE64') }]
                        : [{ value: 'UTF8', label: t('UTF8') }, { value: 'HEX', label: t('HEX') }, { value: 'BASE64', label: t('BASE64') }]
                    }
                    className="text-xs th-text-3 hover:text-indigo-400 font-medium bg-indigo-500/5 px-2 py-1 rounded"
                    menuClassName="left-0 min-w-[6rem]"
                  />
                )}
              </div>
              <button 
                onClick={copyOutput}
                className={`p-1.5 rounded transition-colors ${copied ? 'text-emerald-400 bg-emerald-500/10' : copyError ? 'text-rose-400 bg-rose-500/10' : 'th-text-muted hover:text-emerald-400 hover:bg-emerald-500/10'}`}
                title={copied ? t('Copied!') : copyError ? t('Failed') : t('Copy Output')}
              >
                {copied ? <Check className="w-4 h-4" /> : copyError ? <XCircle className="w-4 h-4" /> : <Copy className="w-4 h-4" />}
              </button>
            </div>
            <div className="flex-1 relative">
              <textarea
                value={output}
                readOnly
                className="absolute inset-0 w-full h-full p-4 bg-transparent border-none focus:ring-0 th-text placeholder:text-gray-500/40 resize-none font-mono text-sm leading-relaxed"
                placeholder="..."
                spellCheck={false}
              />
              {error && (
                <div className="absolute bottom-4 left-4 right-4 p-3 bg-rose-500/10 border border-rose-500/20 text-rose-400 text-sm rounded-lg backdrop-blur-sm shadow-sm break-words">
                  {error}
                </div>
              )}
            </div>
          </div>
          
        </div>
      </div>
    </div>
  );
}
