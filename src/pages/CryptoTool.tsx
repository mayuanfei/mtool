import { useState } from 'react';
import { Lock, Copy, Trash2, ArrowDown, ShieldAlert, Check } from 'lucide-react';
import { useI18n } from '../i18n';
import CryptoJS from 'crypto-js';
import { sm2, sm3, sm4 } from 'sm-crypto';
import JSEncrypt from 'jsencrypt';
import { invoke } from '@tauri-apps/api/core';

type Category = 'hash' | 'symmetric' | 'asymmetric' | 'hq';
type Algorithm = 'MD5' | 'SHA1' | 'SHA256' | 'SM3' | 'AES' | 'DES' | 'SM4' | 'RSA' | 'SM2' | 'HQ_DLL';

function stringToHex(str: string) {
  return Array.from(str).map(c => c.charCodeAt(0).toString(16).padStart(2, '0')).join('');
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
  
  // For Asymmetric
  const [publicKey, setPublicKey] = useState('');
  const [privateKey, setPrivateKey] = useState('');

  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  const categories: { value: Category; label: string }[] = [
    { value: 'hash', label: t('Hash / Digest') },
    { value: 'symmetric', label: t('Symmetric') },
    { value: 'asymmetric', label: t('Asymmetric') },
    { value: 'hq', label: t('HQ DLL') },
  ];

  const algorithms: Record<Category, Algorithm[]> = {
    hash: ['MD5', 'SHA1', 'SHA256', 'SM3'],
    symmetric: ['AES', 'DES', 'SM4'],
    asymmetric: ['RSA', 'SM2'],
    hq: ['HQ_DLL'],
  };

  const handleCategoryChange = (c: Category) => {
    setCategory(c);
    setAlgorithm(algorithms[c][0]);
    setOutput('');
    setError(null);
  };

  const handleAlgorithmChange = (a: Algorithm) => {
    setAlgorithm(a);
    setOutput('');
    setError(null);
  };

  const getCryptoJsMode = () => {
    return mode === 'ECB' ? CryptoJS.mode.ECB : CryptoJS.mode.CBC;
  };

  const handleAction = async (isEncrypt: boolean) => {
    setError(null);
    try {
      let res = '';
      
      // ================= HASH =================
      if (category === 'hash') {
        if (!isEncrypt) throw new Error(t('Hash algorithms cannot be decrypted.'));
        if (algorithm === 'MD5') {
          res = CryptoJS.MD5(input).toString();
        } else if (algorithm === 'SHA1') {
          res = CryptoJS.SHA1(input).toString();
        } else if (algorithm === 'SHA256') {
          res = CryptoJS.SHA256(input).toString();
        } else if (algorithm === 'SM3') {
          res = sm3(input);
        }
      } 
      // ================= SYMMETRIC =================
      else if (category === 'symmetric') {
        if (!cryptoKey) throw new Error(t('Key is required.'));
        
        if (algorithm === 'AES') {
          const cfg: any = { mode: getCryptoJsMode() };
          if (mode === 'CBC') {
            if (!iv) throw new Error(t('IV is required for CBC mode.'));
            cfg.iv = CryptoJS.enc.Utf8.parse(iv);
          }
          const keyUtf8 = CryptoJS.enc.Utf8.parse(cryptoKey);
          
          if (isEncrypt) {
            res = CryptoJS.AES.encrypt(input, keyUtf8, cfg).toString();
          } else {
            const dec = CryptoJS.AES.decrypt(input, keyUtf8, cfg);
            res = dec.toString(CryptoJS.enc.Utf8);
            if (!res) throw new Error(t('Decryption failed. Invalid Key/IV or data.'));
          }
        } 
        else if (algorithm === 'DES') {
          const cfg: any = { mode: getCryptoJsMode() };
          if (mode === 'CBC') {
            if (!iv) throw new Error(t('IV is required for CBC mode.'));
            cfg.iv = CryptoJS.enc.Utf8.parse(iv);
          }
          const keyUtf8 = CryptoJS.enc.Utf8.parse(cryptoKey);
          
          if (isEncrypt) {
            res = CryptoJS.DES.encrypt(input, keyUtf8, cfg).toString();
          } else {
            const dec = CryptoJS.DES.decrypt(input, keyUtf8, cfg);
            res = dec.toString(CryptoJS.enc.Utf8);
            if (!res) throw new Error(t('Decryption failed. Invalid Key/IV or data.'));
          }
        }
        else if (algorithm === 'SM4') {
          // sm-crypto SM4 requires 16 bytes hex string for key/iv by default in some usages, 
          // but we can pass string to the library if we convert it to hex.
          // sm4.encrypt(inArray, keyArray, {mode: 'cbc', iv: ivArray})
          // Simplified text-based SM4 wrapper
          const keyHex = stringToHex(cryptoKey);
          const cfg: any = {};
          if (mode === 'CBC') {
            if (!iv) throw new Error(t('IV is required for CBC mode.'));
            cfg.iv = stringToHex(iv);
            cfg.mode = 'cbc';
          }
          
          if (isEncrypt) {
            res = sm4.encrypt(input, keyHex, cfg);
          } else {
            res = sm4.decrypt(input, keyHex, cfg);
          }
        }
      }
      // ================= ASYMMETRIC =================
      else if (category === 'asymmetric') {
        if (algorithm === 'RSA') {
          const encryptor = new JSEncrypt();
          if (isEncrypt) {
            if (!publicKey) throw new Error(t('Public Key is required for encryption.'));
            encryptor.setPublicKey(publicKey);
            const encoded = encryptor.encrypt(input);
            if (!encoded) throw new Error(t('RSA Encryption failed.'));
            res = encoded;
          } else {
            if (!privateKey) throw new Error(t('Private Key is required for decryption.'));
            encryptor.setPrivateKey(privateKey);
            const decoded = encryptor.decrypt(input);
            if (!decoded) throw new Error(t('RSA Decryption failed.'));
            res = decoded;
          }
        }
        else if (algorithm === 'SM2') {
          if (isEncrypt) {
            if (!publicKey) throw new Error(t('Public Key is required for encryption.'));
            res = sm2.doEncrypt(input, publicKey, 1); // 1 for cipherMode C1C3C2
          } else {
            if (!privateKey) throw new Error(t('Private Key is required for decryption.'));
            res = sm2.doDecrypt(input, privateKey, 1, { output: 'utf8' as any });
          }
        }
      }
      // ================= HQ DLL =================
      else if (category === 'hq') {
        // Call Rust Backend Command
        try {
          res = await invoke('hq_crypto', { 
            action: isEncrypt ? 'encrypt' : 'decrypt',
            payload: input 
          });
        } catch (err: any) {
          throw new Error(`DLL Call Error: ${err}`);
        }
      }

      setOutput(res);
    } catch (err: any) {
      setError(err.message || String(err));
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
    }
  };

  const generateKeys = () => {
    if (algorithm === 'RSA') {
      const encryptor = new JSEncrypt({ default_key_size: '1024' });
      encryptor.getKey();
      setPublicKey(encryptor.getPublicKey());
      setPrivateKey(encryptor.getPrivateKey());
    } else if (algorithm === 'SM2') {
      const keypair = sm2.generateKeyPairHex();
      setPublicKey(keypair.publicKey);
      setPrivateKey(keypair.privateKey);
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

        {/* Configuration Area */}
        {category === 'symmetric' && (
          <div className="grid grid-cols-3 gap-4 shrink-0 border th-border rounded-xl p-4 th-bg-surface-h">
            <div className="flex flex-col gap-1.5 col-span-1">
              <label className="text-xs font-semibold th-text-muted uppercase">{t('Mode')}</label>
              <select 
                value={mode}
                onChange={(e) => setMode(e.target.value)}
                className="w-full px-3 py-2 bg-transparent border th-border rounded-lg th-text text-sm focus:ring-1 focus:ring-indigo-500 outline-none"
              >
                <option value="CBC">CBC</option>
                <option value="ECB">ECB</option>
              </select>
            </div>
            <div className="flex flex-col gap-1.5 col-span-1">
              <label className="text-xs font-semibold th-text-muted uppercase">{t('Key')}</label>
              <input 
                type="text"
                value={cryptoKey}
                onChange={(e) => setCryptoKey(e.target.value)}
                placeholder="Secret Key"
                className="w-full px-3 py-2 bg-transparent border th-border rounded-lg th-text text-sm focus:ring-1 focus:ring-indigo-500 outline-none"
              />
            </div>
            <div className="flex flex-col gap-1.5 col-span-1">
              <label className="text-xs font-semibold th-text-muted uppercase">{t('IV')}</label>
              <input 
                type="text"
                value={iv}
                onChange={(e) => setIv(e.target.value)}
                disabled={mode === 'ECB'}
                placeholder={mode === 'ECB' ? t('Not needed for ECB') : 'Initialization Vector'}
                className="w-full px-3 py-2 bg-transparent border th-border rounded-lg th-text text-sm focus:ring-1 focus:ring-indigo-500 outline-none disabled:opacity-50"
              />
            </div>
          </div>
        )}

        {category === 'asymmetric' && (
          <div className="flex flex-col gap-4 shrink-0 border th-border rounded-xl p-4 th-bg-surface-h">
            <div className="flex justify-between items-center">
              <span className="text-xs font-semibold th-text-muted uppercase">{t('Keys Configuration')}</span>
              <button 
                onClick={generateKeys}
                className="text-xs text-indigo-400 hover:text-indigo-300 font-medium"
              >
                {t('Auto Generate Key Pair')}
              </button>
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div className="flex flex-col gap-1.5 col-span-1">
                <label className="text-xs th-text-3">{t('Public Key')}</label>
                <textarea 
                  value={publicKey}
                  onChange={(e) => setPublicKey(e.target.value)}
                  placeholder="-----BEGIN PUBLIC KEY-----..."
                  className="w-full h-24 p-3 bg-transparent border th-border rounded-lg th-text text-xs font-mono resize-none focus:ring-1 focus:ring-indigo-500 outline-none"
                />
              </div>
              <div className="flex flex-col gap-1.5 col-span-1">
                <label className="text-xs th-text-3">{t('Private Key')}</label>
                <textarea 
                  value={privateKey}
                  onChange={(e) => setPrivateKey(e.target.value)}
                  placeholder="-----BEGIN PRIVATE KEY-----..."
                  className="w-full h-24 p-3 bg-transparent border th-border rounded-lg th-text text-xs font-mono resize-none focus:ring-1 focus:ring-indigo-500 outline-none"
                />
              </div>
            </div>
          </div>
        )}

        {category === 'hq' && (
          <div className="flex items-center gap-3 shrink-0 border border-indigo-500/20 bg-indigo-500/5 rounded-xl p-4 text-sm text-indigo-400">
            <ShieldAlert className="w-5 h-5 flex-shrink-0" />
            <span>{t('This feature requires the HQ DLL installed in C:\\Windows\\System32. Only available on Windows environments.')}</span>
          </div>
        )}

        <div className="flex-1 flex gap-4 min-h-0">
          
          {/* Input Panel */}
          <div className="flex-1 flex flex-col min-h-0 border th-border rounded-xl overflow-hidden shadow-sm th-bg-card">
            <div className="px-4 py-3 border-b th-border th-bg-surface-h flex items-center justify-between">
              <span className="font-semibold text-sm th-text-2 uppercase tracking-tight">{t('Input')}</span>
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
            <button
              onClick={() => handleAction(true)}
              className="flex flex-col items-center justify-center gap-1 w-24 py-3 bg-indigo-600 hover:bg-indigo-700 text-white rounded-xl shadow-lg transition-all active:scale-95"
            >
              <span className="font-bold text-sm">{category === 'hash' ? t('Hash') : t('Encrypt')}</span>
              <ArrowDown className="w-4 h-4 -rotate-90" />
            </button>
            {category !== 'hash' && (
              <button
                onClick={() => handleAction(false)}
                className="flex flex-col items-center justify-center gap-1 w-24 py-3 th-bg-surface th-hover-surface border th-border th-text-2 rounded-xl shadow-sm transition-all active:scale-95"
              >
                <ArrowDown className="w-4 h-4 -rotate-90" />
                <span className="font-bold text-sm">{t('Decrypt')}</span>
              </button>
            )}
          </div>

          {/* Output Panel */}
          <div className="flex-1 flex flex-col min-h-0 border th-border rounded-xl overflow-hidden shadow-sm th-bg-card">
            <div className="px-4 py-3 border-b th-border th-bg-surface-h flex items-center justify-between">
              <span className="font-semibold text-sm th-text-2 uppercase tracking-tight">{t('Output')}</span>
              <button 
                onClick={copyOutput}
                className={`p-1.5 rounded transition-colors ${copied ? 'text-emerald-400 bg-emerald-500/10' : 'th-text-muted hover:text-emerald-400 hover:bg-emerald-500/10'}`}
                title={copied ? t('Copied!') : t('Copy Output')}
              >
                {copied ? <Check className="w-4 h-4" /> : <Copy className="w-4 h-4" />}
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
