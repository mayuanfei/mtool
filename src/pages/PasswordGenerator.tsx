import { Copy, RefreshCcw, Check, Key, XCircle } from 'lucide-react';
import { useState, useEffect, useCallback } from 'react';
import { useI18n, TranslationKey } from '../i18n';

const Toggle = ({ checked, onChange }: { checked: boolean; onChange: () => void }) => {
  return (
    <button
      onClick={onChange}
      className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-indigo-500 focus:ring-offset-2 th-ring-offset ${
        checked ? 'bg-indigo-500' : 'th-bg-surface'
      }`}
    >
      <span
        className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
          checked ? 'translate-x-6' : 'translate-x-1'
        }`}
      />
    </button>
  );
};

interface HistoryItem {
  id: string;
  timestamp: string;
  password: string;
  strength: 'STRONG' | 'GOOD' | 'FAIR' | 'WEAK';
}

const UPPERCASE = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ';
const LOWERCASE = 'abcdefghijklmnopqrstuvwxyz';
const NUMBERS = '0123456789';

function unbiasedRandom(max: number): number {
  const limit = Math.floor(0x100000000 / max) * max;
  const arr = new Uint32Array(1);
  let val: number;
  do {
    window.crypto.getRandomValues(arr);
    val = arr[0];
  } while (val >= limit);
  return val % max;
}

export function PasswordGenerator() {
  const { t } = useI18n();
  const [passwords, setPasswords] = useState<string[]>([]);
  const [length, setLength] = useState(() => {
    const saved = localStorage.getItem('mtool_pwd_length');
    const val = saved ? parseInt(saved, 10) : 16;
    return isNaN(val) ? 16 : Math.max(4, Math.min(128, val));
  });
  const [generateCount, setGenerateCount] = useState(() => {
    const saved = localStorage.getItem('mtool_pwd_count');
    const val = saved ? parseInt(saved, 10) : 1;
    return isNaN(val) ? 1 : Math.max(1, Math.min(100, val));
  });
  const [useUpper, setUseUpper] = useState(() =>
    localStorage.getItem('mtool_pwd_upper') !== 'false');
  const [useLower, setUseLower] = useState(() =>
    localStorage.getItem('mtool_pwd_lower') !== 'false');
  const [useNumbers, setUseNumbers] = useState(() =>
    localStorage.getItem('mtool_pwd_numbers') !== 'false');
  const [useSymbols, setUseSymbols] = useState(() =>
    localStorage.getItem('mtool_pwd_use_symbols') !== 'false');
  const [customSymbols, setCustomSymbols] = useState(() => {
    return localStorage.getItem('mtool_pwd_symbols') || '!@#$%^&*';
  });
  const [excludeChars, setExcludeChars] = useState(() => {
    return localStorage.getItem('mtool_pwd_exclude') || '';
  });
  const [history, setHistory] = useState<HistoryItem[]>(() => {
    const saved = localStorage.getItem('mtool_pwd_history');
    if (saved) {
      try {
        const parsed = JSON.parse(saved);
        return Array.isArray(parsed) ? parsed.slice(0, 5) : [];
      } catch (e) {
        return [];
      }
    }
    return [];
  });
  const [isCopied, setIsCopied] = useState(false);
  const [copyError, setCopyError] = useState(false);
  const [charsetEmpty, setCharsetEmpty] = useState(false);
  const [historyCopiedId, setHistoryCopiedId] = useState<string | null>(null);
  const [historyCopyErrorId, setHistoryCopyErrorId] = useState<string | null>(null);

  const calculateStrength = useCallback((): 'STRONG' | 'GOOD' | 'FAIR' | 'WEAK' => {
    let score = 0;
    if (useUpper) score += 1;
    if (useLower) score += 1;
    if (useNumbers) score += 1;
    if (useSymbols && customSymbols.length > 0) score += 1;

    if (length < 8) return 'WEAK';
    if (length < 12) {
      if (score >= 3) return 'GOOD';
      if (score === 2) return 'FAIR';
      return 'WEAK';
    }
    if (score === 4) return 'STRONG';
    if (score === 3) return 'GOOD';
    if (score === 2) return 'FAIR';
    return 'WEAK';
  }, [useUpper, useLower, useNumbers, useSymbols, customSymbols, length]);

  const generatePassword = useCallback((addToHistory = true) => {
    let charset = '';
    if (useUpper) charset += UPPERCASE;
    if (useLower) charset += LOWERCASE;
    if (useNumbers) charset += NUMBERS;
    if (useSymbols) charset += customSymbols;

    if (excludeChars) {
      const excludeSet = new Set(excludeChars);
      charset = [...charset].filter(c => !excludeSet.has(c)).join('');
    }

    if (!charset) {
      setPasswords([]);
      setCharsetEmpty(true);
      return;
    }

    setCharsetEmpty(false);

    const newPasswords: string[] = [];
    const historyAdditions: HistoryItem[] = [];
    
    const now = new Date();
    const pad = (n: number) => String(n).padStart(2, '0');
    const timeStr = `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())} ${pad(now.getHours())}:${pad(now.getMinutes())}:${pad(now.getSeconds())}`;

    for (let c = 0; c < generateCount; c++) {
      let generated = '';
      for (let i = 0; i < length; i++) {
        generated += charset[unbiasedRandom(charset.length)];
      }
      newPasswords.push(generated);
      
      if (addToHistory && generated) {
        historyAdditions.push({
          id: crypto.randomUUID(),
          timestamp: timeStr,
          password: generated,
          strength: calculateStrength()
        });
      }
    }

    setPasswords(newPasswords);

    if (addToHistory && historyAdditions.length > 0) {
      setHistory(prev => [...historyAdditions, ...prev].slice(0, 5)); // Keep last 5
    }
  }, [length, useUpper, useLower, useNumbers, useSymbols, customSymbols, excludeChars, generateCount, calculateStrength]);



  // Save history to localStorage
  useEffect(() => {
    localStorage.setItem('mtool_pwd_history', JSON.stringify(history));
  }, [history]);

  // Save custom symbols to localStorage
  useEffect(() => {
    localStorage.setItem('mtool_pwd_symbols', customSymbols);
  }, [customSymbols]);

  // Save exclude chars to localStorage
  useEffect(() => {
    localStorage.setItem('mtool_pwd_exclude', excludeChars);
  }, [excludeChars]);

  // Save length to localStorage
  useEffect(() => {
    localStorage.setItem('mtool_pwd_length', length.toString());
  }, [length]);

  // Save generate count to localStorage
  useEffect(() => {
    localStorage.setItem('mtool_pwd_count', generateCount.toString());
  }, [generateCount]);

  // Save boolean toggles to localStorage
  useEffect(() => {
    localStorage.setItem('mtool_pwd_upper', useUpper.toString());
  }, [useUpper]);
  useEffect(() => {
    localStorage.setItem('mtool_pwd_lower', useLower.toString());
  }, [useLower]);
  useEffect(() => {
    localStorage.setItem('mtool_pwd_numbers', useNumbers.toString());
  }, [useNumbers]);
  useEffect(() => {
    localStorage.setItem('mtool_pwd_use_symbols', useSymbols.toString());
  }, [useSymbols]);

  // When options change, generate new one automatically (without adding to history for slider drag)
  useEffect(() => {
    generatePassword(false);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [length, useUpper, useLower, useNumbers, useSymbols, customSymbols, excludeChars, generateCount]);

  const handleCopy = async (text: string, isHistoryId: string | null = null) => {
    if (!text) return;
    try {
      await navigator.clipboard.writeText(text);
      if (isHistoryId) {
        setHistoryCopiedId(isHistoryId);
        setTimeout(() => setHistoryCopiedId(null), 2000);
      } else {
        setIsCopied(true);
        setTimeout(() => setIsCopied(false), 2000);
      }
    } catch (e) {
      console.error(e);
      if (isHistoryId) {
        setHistoryCopyErrorId(isHistoryId);
        setTimeout(() => setHistoryCopyErrorId(null), 2000);
      } else {
        setCopyError(true);
        setTimeout(() => setCopyError(false), 2000);
      }
    }
  };

  const strength = calculateStrength();
  
  const getStrengthColor = (s: string) => {
    if (s === 'STRONG') return 'bg-emerald-500';
    if (s === 'GOOD') return 'bg-blue-500';
    if (s === 'FAIR') return 'bg-amber-500';
    return 'bg-rose-500';
  };
  
  const getStrengthTextColor = (s: string) => {
    if (s === 'STRONG') return 'text-emerald-500';
    if (s === 'GOOD') return 'text-blue-500';
    if (s === 'FAIR') return 'text-amber-500';
    return 'text-rose-500';
  };

  const getStrengthBars = () => {
    let bars = 1;
    if (strength === 'FAIR') bars = 2;
    if (strength === 'GOOD') bars = 3;
    if (strength === 'STRONG') bars = 4;
    
    return (
      <div className="flex gap-2 flex-1 mr-4">
        {[1, 2, 3, 4].map(i => (
          <div 
            key={i} 
            className={`h-1.5 rounded-full flex-1 transition-colors ${
              i <= bars ? getStrengthColor(strength) : 'th-bg-surface'
            } ${i <= bars && strength === 'STRONG' ? 'shadow-[0_0_10px_rgba(16,185,129,0.5)]' : ''}`}
          ></div>
        ))}
      </div>
    );
  };

  const maskPassword = (pwd: string) => {
    if (pwd.length <= 4) return pwd;
    return '*'.repeat(8) + '_' + pwd.slice(-3);
  };

  return (
    <div className="w-full h-full flex flex-col min-h-0">
      <div className="flex justify-between items-center mb-6 border-b th-border pb-4 shrink-0">
        <h2 className="th-text font-semibold text-lg flex items-center gap-2">
          <Key className="w-5 h-5 text-indigo-400" />
          {t('Password Generator')}
        </h2>
      </div>

      <div className="flex-1 flex flex-col min-h-0 overflow-y-auto pr-2 pb-6 space-y-6">
        
        {/* Main Generator Card */}
        <section className="th-bg-card border th-border rounded-xl p-6 shadow-2xl shrink-0">
          
          {/* Password Display */}
          <div className="th-bg-input border th-border rounded-lg p-5 mb-8">
            <div className="flex items-start justify-between mb-6">
              <div className={`font-mono tracking-widest th-text overflow-y-auto max-h-40 mr-4 flex flex-col gap-2 w-full ${passwords.length > 1 ? 'text-xl' : 'text-3xl whitespace-nowrap overflow-hidden text-ellipsis'}`}>
                {passwords.length > 0 ? passwords.map((p, i) => (
                  <div key={i} className="break-all">{p}</div>
                )) : (
                  charsetEmpty
                    ? <span className="text-sm text-amber-500 font-sans tracking-normal">{t('Please select at least one character type')}</span>
                    : '---'
                )}
              </div>
              <div className="flex items-center gap-3 shrink-0 mt-1">
                <button 
                  onClick={() => generatePassword(true)}
                  className="p-2 th-text-3 hover:th-text th-hover-surface rounded-lg transition-colors focus:outline-none"
                  title={t('Regenerate')}
                >
                  <RefreshCcw className="w-5 h-5" />
                </button>
                <button 
                  onClick={() => handleCopy(passwords.join('\n'))}
                  className={`px-4 py-2 rounded-lg font-medium transition-colors flex items-center gap-2 focus:outline-none shadow-lg ${
                    copyError ? 'bg-red-500/20 text-red-400 border border-red-500/30 shadow-red-500/10' :
                    isCopied ? 'bg-emerald-500/20 text-emerald-400 border border-emerald-500/30 shadow-emerald-500/10' :
                    'bg-indigo-600 hover:bg-indigo-500 text-white shadow-indigo-600/20'
                  }`}
                >
                  {copyError ? <XCircle className="w-4 h-4" /> : isCopied ? <Check className="w-4 h-4" /> : <Copy className="w-4 h-4" />}
                  {copyError ? t('Failed') : isCopied ? t('Copied!') : t('Copy')}
                </button>
              </div>
            </div>
            
            <div className="flex items-center">
              {getStrengthBars()}
              <span className={`text-[11px] font-bold tracking-widest uppercase ${getStrengthTextColor(strength)}`}>
                {t(strength as TranslationKey)}
              </span>
            </div>
          </div>

          {/* Controls */}
          <div className="mb-8">
            <div className="flex justify-between items-center mb-4">
              <span className="text-sm font-bold th-text-2">{t('Password Length')}</span>
              <div className="flex items-center gap-3">
                <input 
                  type="number" 
                  min="4" max="128"
                  value={length}
                  onChange={(e) => {
                    let val = parseInt(e.target.value);
                    if (isNaN(val)) val = 4;
                    if (val > 128) val = 128;
                    setLength(val);
                  }}
                  onBlur={() => {
                    if (length < 4) setLength(4);
                  }}
                  className="w-16 th-bg-input border th-border-subtle th-text-2 text-sm font-mono rounded px-2 py-1 focus:outline-none focus:border-indigo-500 text-center"
                />
              </div>
            </div>
            <input 
              type="range" min="4" max="128" step="1"
              value={length}
              onChange={(e) => setLength(parseInt(e.target.value))}
              className="w-full cursor-pointer pwd-slider"
            />
          </div>

          <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
            <div className="th-bg-input-alt border th-border rounded-lg p-4 flex items-center justify-between shadow-inner">
              <div>
                <p className="text-sm font-bold th-text-2 mb-1">{t('Uppercase Letters')}</p>
                <p className="text-xs th-text-muted font-mono">{t('A-Z')}</p>
              </div>
              <Toggle checked={useUpper} onChange={() => setUseUpper(!useUpper)} />
            </div>
            
            <div className="th-bg-input-alt border th-border rounded-lg p-4 flex items-center justify-between shadow-inner">
              <div>
                <p className="text-sm font-bold th-text-2 mb-1">{t('Lowercase Letters')}</p>
                <p className="text-xs th-text-muted font-mono">{t('a-z')}</p>
              </div>
              <Toggle checked={useLower} onChange={() => setUseLower(!useLower)} />
            </div>

            <div className="th-bg-input-alt border th-border rounded-lg p-4 flex items-center justify-between shadow-inner">
              <div>
                <p className="text-sm font-bold th-text-2 mb-1">{t('Numbers')}</p>
                <p className="text-xs th-text-muted font-mono">{t('0-9')}</p>
              </div>
              <Toggle checked={useNumbers} onChange={() => setUseNumbers(!useNumbers)} />
            </div>

            <div className="th-bg-input-alt border th-border rounded-lg p-4 flex items-center justify-between shadow-inner">
              <div className="flex-1 mr-4">
                <p className="text-sm font-bold th-text-2 mb-1">{t('Symbols')}</p>
                <input 
                  type="text" 
                  value={customSymbols}
                  disabled={!useSymbols}
                  onChange={(e) => setCustomSymbols(e.target.value)}
                  className={`w-full th-bg-card border th-border-subtle th-text-2 text-xs font-mono rounded px-2 py-1 focus:outline-none focus:border-indigo-500 ${!useSymbols ? 'opacity-50 cursor-not-allowed' : ''}`}
                />
              </div>
              <Toggle checked={useSymbols} onChange={() => setUseSymbols(!useSymbols)} />
            </div>
          </div>

          <div className="grid grid-cols-1 md:grid-cols-2 gap-4 mt-4">
            <div className="th-bg-input-alt border th-border rounded-lg p-4 shadow-inner">
              <p className="text-sm font-bold th-text-2 mb-2">{t('Exclude Characters')}</p>
              <input 
                type="text" 
                value={excludeChars}
                onChange={(e) => setExcludeChars(e.target.value)}
                placeholder={t('e.g. iIl1Oo0')}
                className="w-full th-bg-card border th-border-subtle th-text-2 text-xs font-mono rounded px-3 py-2 focus:outline-none focus:border-indigo-500"
              />
            </div>
            <div className="th-bg-input-alt border th-border rounded-lg p-4 shadow-inner">
              <p className="text-sm font-bold th-text-2 mb-2">{t('Generate Count')}</p>
              <input 
                type="number" 
                min="1" max="100"
                value={generateCount}
                onChange={(e) => {
                  const val = parseInt(e.target.value);
                  setGenerateCount(isNaN(val) ? 1 : Math.max(1, Math.min(100, val)));
                }}
                className="w-full th-bg-card border th-border-subtle th-text-2 text-sm font-mono rounded px-3 py-1.5 focus:outline-none focus:border-indigo-500"
              />
            </div>
          </div>
        </section>

        {/* History Table */}
        {history.length > 0 && (
          <section className="th-bg-card border th-border rounded-xl overflow-hidden shadow-2xl flex flex-col min-h-[16rem] flex-1">
            <div className="px-6 py-4 border-b th-border th-bg-surface-h flex justify-between items-center shrink-0">
              <span className="text-[11px] font-bold th-text-3 uppercase tracking-widest">{t('Generation History')}</span>
              <span className="text-[10px] th-text-muted font-mono">{t('Last 5 items')}</span>
            </div>
            <div className="overflow-y-auto flex-1 min-h-0">
              <table className="w-full text-left text-sm th-text-3">
                <thead className="text-[10px] uppercase font-bold tracking-widest th-text-muted border-b th-border th-bg-input-alt sticky top-0 z-10">
                  <tr>
                    <th className="px-6 py-3">{t('TIMESTAMP')}</th>
                    <th className="px-6 py-3">{t('PREVIEW')}</th>
                    <th className="px-6 py-3">{t('STRENGTH')}</th>
                    <th className="px-6 py-3 text-right">{t('ACTION')}</th>
                  </tr>
                </thead>
                <tbody className="divide-y th-divide">
                  {history.map((item) => (
                    <tr key={item.id} className="th-hover-surface transition-colors">
                      <td className="px-6 py-3.5 font-mono text-xs whitespace-nowrap">{item.timestamp}</td>
                      <td className="px-6 py-3.5 font-mono th-text-2">{maskPassword(item.password)}</td>
                      <td className="px-6 py-3.5 whitespace-nowrap">
                        <span className={`px-2 py-1 rounded text-[10px] font-bold uppercase ${
                          item.strength === 'STRONG' ? 'bg-emerald-500/10 text-emerald-400' :
                          item.strength === 'GOOD' ? 'bg-blue-500/10 text-blue-400' :
                          item.strength === 'FAIR' ? 'bg-amber-500/10 text-amber-400' :
                          'bg-rose-500/10 text-rose-400'
                        }`}>
                          {t(item.strength as TranslationKey)}
                        </span>
                      </td>
                      <td className="px-6 py-3.5 text-right whitespace-nowrap">
                        <button 
                          onClick={() => handleCopy(item.password, item.id)}
                          className="p-2 th-text-muted hover:th-text th-hover-surface rounded transition-colors focus:outline-none inline-flex"
                          title={t('Copy')}
                        >
                          {historyCopyErrorId === item.id ? <XCircle className="w-4 h-4 text-rose-400" /> : historyCopiedId === item.id ? <Check className="w-4 h-4 text-emerald-400" /> : <Copy className="w-4 h-4" />}
                        </button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </section>
        )}

      </div>
    </div>
  );
}
