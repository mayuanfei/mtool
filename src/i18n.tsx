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
    'User profile and preferences.': 'User profile and preferences.',
    'Coming soon': 'Coming soon',
    
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
    'Length': 'Length',
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
    'Exclude Characters': 'Exclude Characters',
    'e.g. iIl1Oo0': 'e.g. iIl1Oo0',
    'Generate Count': 'Generate Count',
    'TIMESTAMP': 'TIMESTAMP',
    'PREVIEW': 'PREVIEW',
    'STRENGTH': 'STRENGTH',
    'ACTION': 'ACTION',
    'Create secure passwords.': 'Create secure passwords.',
    
    // SqlInBuilder
    'SQL IN Builder': 'SQL IN Builder',
    'Convert a column of values into a SQL IN clause string.': 'Convert a column of values into a SQL IN clause string.',
    'Quote Style': 'Quote Style',
    'Single Quote': 'Single Quote',
    'Double Quote': 'Double Quote',
    'No Quote': 'No Quote',
    'Input Values': 'Input Values',
    'SQL Output': 'SQL Output',
    'Paste one value per line...': 'Paste one value per line...',
    'items': 'items',
    'duplicates removed': 'duplicates removed',
    'SQL IN clause will appear here...': 'SQL IN clause will appear here...',
    'unique values': 'unique values',
    'SQL Preview': 'SQL Preview',
    'Build SQL IN clause from column values.': 'Build SQL IN clause from column values.',
    
    // Settings
    'Configure MTOOL behaviors and active utilities.': 'Configure MTOOL behaviors and active utilities.',
    'Utility Configuration': 'Utility Configuration',
    'Parse, validate, and beautify raw JSON payloads.': 'Parse, validate, and beautify raw JSON payloads.',
    'Generate scannable QR codes from string inputs.': 'Generate scannable QR codes from string inputs.',
    'General Settings': 'General Settings',
    'Language': 'Language',
    'Select application interface language.': 'Select application interface language.',
    'Select a tool from the sidebar': 'Select a tool from the sidebar',

    // MarkdownEditor
    'Markdown Editor': 'Markdown Editor',
    'View and edit Markdown files with live preview.': 'View and edit Markdown files with live preview.',
    'Editor': 'Editor',
    'Start writing Markdown...': 'Start writing Markdown...',
    'Preview will appear here...': 'Preview will appear here...',
    'Open': 'Open',
    'Save': 'Save',
    'Save As': 'Save As',
    'Untitled': 'Untitled',
    'Modified': 'Modified',
    'Words': 'Words',
    'Edit Mode': 'Edit Mode',
    'Split View': 'Split View',
    'Preview Mode': 'Preview Mode',
    'Collapse Sidebar': 'Collapse Sidebar',
    'Expand Sidebar': 'Expand Sidebar',
    'Release to open file': 'Release to open file',

    // FileSearch
    'File Search': 'File Search',
    'Search files by name, size, or content across indexed directories.': 'Search files by name, size, or content across indexed directories.',
    'Try: *.yaml   report draft   size:>10MB   content:"api_key"': 'Try: *.yaml   report draft   size:>10MB   content:"api_key"',
    'glob': 'glob',
    'AND': 'AND',
    'size': 'size',
    'content': 'content',
    'Index': 'Index',
    'Indexing...': 'Indexing...',
    'files indexed': 'files indexed',
    'Re-index': 'Re-index',
    'Add Directory': 'Add Directory',
    'No directories indexed. Click "Add Directory" to get started.': 'No directories indexed. Click "Add Directory" to get started.',
    'Remove': 'Remove',
    'Results': 'Results',
    'Searching...': 'Searching...',
    'No results': 'No results',
    'matches': 'matches',
    'Index a directory first, then search.': 'Index a directory first, then search.',
    'No files match your query.': 'No files match your query.',
    'Search and find files by name, size, or content.': 'Search and find files by name, size, or content.'
  },
  zh: {
    // Sidebar
    'Tools': '工具',
    'JSON Formatter': 'JSON 格式化',
    'Text to QR': '文本转二维码',
    'System': '系统',
    'Settings': '设置',
    'User': '用户',
    'User profile and preferences.': '用户资料与偏好设置。',
    'Coming soon': '即将推出',
    
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
    'Length': '长度',
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
    'Exclude Characters': '排除字符',
    'e.g. iIl1Oo0': '例如：iIl1Oo0',
    'Generate Count': '生成数量',
    'TIMESTAMP': '时间',
    'PREVIEW': '预览',
    'STRENGTH': '强度',
    'ACTION': '操作',
    'Create secure passwords.': '创建安全的密码。',
    
    // SqlInBuilder
    'SQL IN Builder': 'SQL IN 构建器',
    'Convert a column of values into a SQL IN clause string.': '将一列值转换为 SQL IN 子句字符串。',
    'Quote Style': '引号风格',
    'Single Quote': '单引号',
    'Double Quote': '双引号',
    'No Quote': '无引号',
    'Input Values': '输入值',
    'SQL Output': 'SQL 输出',
    'Paste one value per line...': '每行粘贴一个值...',
    'items': '项',
    'duplicates removed': '个重复项已去除',
    'SQL IN clause will appear here...': 'SQL IN 子句将显示在此处...',
    'unique values': '个唯一值',
    'SQL Preview': 'SQL 预览',
    'Build SQL IN clause from column values.': '将一列值组装为 SQL IN 子句。',
    
    // Settings
    'Configure MTOOL behaviors and active utilities.': '配置 MTOOL 的行为和启用的工具。',
    'Utility Configuration': '工具配置',
    'Parse, validate, and beautify raw JSON payloads.': '解析、验证并美化原始 JSON 文本。',
    'Generate scannable QR codes from string inputs.': '将文本转换为可扫描的二维码图片。',
    'General Settings': '常规设置',
    'Language': '语言',
    'Select application interface language.': '选择应用程序界面语言。',
    'Select a tool from the sidebar': '请从左侧边栏选择一个工具',

    // MarkdownEditor
    'Markdown Editor': 'Markdown 编辑器',
    'View and edit Markdown files with live preview.': '查看和编辑 Markdown 文件，支持实时预览。',
    'Editor': '编辑器',
    'Start writing Markdown...': '开始编写 Markdown...',
    'Preview will appear here...': '预览将显示在此处...',
    'Open': '打开',
    'Save': '保存',
    'Save As': '另存为',
    'Untitled': '未命名',
    'Modified': '已修改',
    'Words': '单词',
    'Edit Mode': '编辑模式',
    'Split View': '分栏视图',
    'Preview Mode': '预览模式',
    'Collapse Sidebar': '收起侧边栏',
    'Expand Sidebar': '展开侧边栏',
    'Release to open file': '释放以打开文件',

    // FileSearch
    'File Search': '文件搜索',
    'Search files by name, size, or content across indexed directories.': '在已索引的目录中按文件名、大小或内容搜索文件。',
    'Try: *.yaml   report draft   size:>10MB   content:"api_key"': '试试: *.yaml   报告 草稿   size:>10MB   content:"密钥"',
    'glob': '通配符',
    'AND': '且',
    'size': '大小',
    'content': '内容',
    'Index': '索引',
    'Indexing...': '正在建立索引...',
    'files indexed': '个文件已索引',
    'Re-index': '重新索引',
    'Add Directory': '添加目录',
    'No directories indexed. Click "Add Directory" to get started.': '未添加目录，点击"添加目录"开始建立索引。',
    'Remove': '移除',
    'Results': '搜索结果',
    'Searching...': '搜索中...',
    'No results': '无结果',
    'matches': '个匹配',
    'Index a directory first, then search.': '请先添加目录建立索引，然后再搜索。',
    'No files match your query.': '没有文件匹配您的查询。',
    'Search and find files by name, size, or content.': '按文件名、大小或内容搜索文件。'
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
