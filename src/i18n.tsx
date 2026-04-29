import React, { createContext, useContext, useState, useEffect } from 'react';

type Language = 'en' | 'zh';

const translations = {
  en: {
    // Sidebar
    'Tools': 'Tools',
    'JSON Formatter': 'JSON Formatter',
    'Text to QR': 'Text to QR',
    'System': 'System',
    'Settings': 'Settings',
    'User': 'User',
    
    // JsonFormatter
    'Clear': 'Clear',
    'Minify': 'Minify',
    'Format JSON': 'Format JSON',
    'Raw Input': 'Raw Input',
    'Formatted Output': 'Formatted Output',
    'Copy': 'Copy',
    'Copied!': 'Copied!',
    'Paste raw JSON here...': 'Paste raw JSON here...',
    'Formatted output will appear here...': 'Formatted output will appear here...',
    'System Ready': 'System Ready',
    'Lines': 'Lines',
    'chars': 'chars',
    
    // TextToQr
    'Raw Payload': 'Raw Payload',
    'Enter URL, text, or JSON payload...': 'Enter URL, text, or JSON payload...',
    'Redundancy Level': 'Redundancy Level',
    'Matrix Resolution': 'Matrix Resolution',
    'Chromatic Injection': 'Chromatic Injection',
    'HEX': 'HEX',
    'Preview': 'Preview',
    'Format: PNG': 'Format: PNG',
    'Dimensions': 'Dimensions',
    'Copy Image': 'Copy Image',
    'Download': 'Download',
    'Enter payload to generate': 'Enter payload to generate',
    
    // PasswordGenerator
    'Password Generator': 'Password Generator',
    'Generate secure, random passwords for your applications.': 'Generate secure, random passwords for your applications.',
    'STRONG': 'STRONG',
    'GOOD': 'GOOD',
    'FAIR': 'FAIR',
    'MEDIUM': 'MEDIUM',
    'WEAK': 'WEAK',
    'Password Length': 'Password Length',
    'Uppercase Letters': 'Uppercase Letters',
    'A-Z': 'A-Z',
    'Lowercase Letters': 'Lowercase Letters',
    'a-z': 'a-z',
    'Numbers': 'Numbers',
    '0-9': '0-9',
    'Symbols': 'Symbols',
    '!@#$%^&*': '!@#$%^&*',
    'TIMESTAMP': 'TIMESTAMP',
    'PREVIEW': 'PREVIEW',
    'STRENGTH': 'STRENGTH',
    'ACTION': 'ACTION',
    'Create secure passwords.': 'Create secure passwords.',
    
    // Settings
    'Configure MTOOL behaviors and active utilities.': 'Configure MTOOL behaviors and active utilities.',
    'Utility Configuration': 'Utility Configuration',
    'Parse, validate, and beautify raw JSON payloads.': 'Parse, validate, and beautify raw JSON payloads.',
    'Generate scannable QR codes from string inputs.': 'Generate scannable QR codes from string inputs.',
    'General Settings': 'General Settings',
    'Language': 'Language',
    'Select application interface language.': 'Select application interface language.',
    'Select a tool from the sidebar': 'Select a tool from the sidebar'
  },
  zh: {
    // Sidebar
    'Tools': '工具',
    'JSON Formatter': 'JSON 格式化',
    'Text to QR': '文本转二维码',
    'System': '系统',
    'Settings': '设置',
    'User': '用户',
    
    // JsonFormatter
    'Clear': '清空',
    'Minify': '压缩',
    'Format JSON': '格式化 JSON',
    'Raw Input': '原始输入',
    'Formatted Output': '格式化输出',
    'Copy': '复制',
    'Copied!': '已复制!',
    'Paste raw JSON here...': '在此粘贴原始 JSON...',
    'Formatted output will appear here...': '格式化后的输出将显示在此处...',
    'System Ready': '系统就绪',
    'Lines': '行数',
    'chars': '字符',
    
    // TextToQr
    'Raw Payload': '原始文本',
    'Enter URL, text, or JSON payload...': '输入网址、文本或 JSON...',
    'Redundancy Level': '容错级别',
    'Matrix Resolution': '图像分辨率',
    'Chromatic Injection': '颜色配置',
    'HEX': 'HEX',
    'Preview': '预览',
    'Format: PNG': '格式: PNG',
    'Dimensions': '尺寸',
    'Copy Image': '复制图片',
    'Download': '下载',
    'Enter payload to generate': '输入文本以生成',
    
    // PasswordGenerator
    'Password Generator': '密码生成器',
    'Generate secure, random passwords for your applications.': '为您的应用程序生成安全的随机密码。',
    'STRONG': '强',
    'GOOD': '中等',
    'FAIR': '较弱',
    'MEDIUM': '中',
    'WEAK': '弱',
    'Password Length': '密码长度',
    'Uppercase Letters': '大写字母',
    'A-Z': 'A-Z',
    'Lowercase Letters': '小写字母',
    'a-z': 'a-z',
    'Numbers': '数字',
    '0-9': '0-9',
    'Symbols': '特殊符号',
    '!@#$%^&*': '!@#$%^&*',
    'TIMESTAMP': '时间',
    'PREVIEW': '预览',
    'STRENGTH': '强度',
    'ACTION': '操作',
    'Create secure passwords.': '创建安全的密码。',
    
    // Settings
    'Configure MTOOL behaviors and active utilities.': '配置 MTOOL 的行为和启用的工具。',
    'Utility Configuration': '工具配置',
    'Parse, validate, and beautify raw JSON payloads.': '解析、验证并美化原始 JSON 文本。',
    'Generate scannable QR codes from string inputs.': '将文本转换为可扫描的二维码图片。',
    'General Settings': '常规设置',
    'Language': '语言',
    'Select application interface language.': '选择应用程序界面语言。',
    'Select a tool from the sidebar': '请从左侧边栏选择一个工具'
  }
};

type I18nContextType = {
  language: Language;
  setLanguage: (lang: Language) => void;
  t: (key: keyof typeof translations.en) => string;
};

const I18nContext = createContext<I18nContextType | undefined>(undefined);

export function I18nProvider({ children }: { children: React.ReactNode }) {
  const [language, setLanguage] = useState<Language>(() => {
    const saved = localStorage.getItem('mtool_language');
    return (saved === 'zh' || saved === 'en') ? saved : 'en';
  });

  useEffect(() => {
    localStorage.setItem('mtool_language', language);
  }, [language]);

  const t = (key: keyof typeof translations.en) => {
    return translations[language][key] || key;
  };

  return (
    <I18nContext.Provider value={{ language, setLanguage, t }}>
      {children}
    </I18nContext.Provider>
  );
}

export function useI18n() {
  const context = useContext(I18nContext);
  if (!context) throw new Error('useI18n must be used within I18nProvider');
  return context;
}
