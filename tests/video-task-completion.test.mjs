import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { runInNewContext } from 'node:vm';

// 直接执行 Rust 内嵌的采集脚本，覆盖目录和普通专题两条路径，不复制判定规则。
const source = readFileSync(new URL('../src-tauri/src/video_tasks.rs', import.meta.url), 'utf8');
const capture = source.split('fn capture_script(')[1].split('"##;')[0];
const helpers = capture.slice(capture.indexOf('const clean ='), capture.indexOf('const isPhaseOrSectionHeader ='));
const progressBlocks = [...capture.matchAll(/const progressMatch = text\.match[^]*?const progress = completed[^;]+;/g)];
assert.equal(progressBlocks.length, 2, '目录和普通专题都应有进度判定');

const readers = [
  ['完成图标识别', runInNewContext(`${helpers}\n(element) => isElementCompleted(element)`)],
  ...progressBlocks.map(([code], index) => [
    index === 0 ? '章节目录采集' : '普通专题采集',
    runInNewContext(`${helpers}\n(element) => {
      const item = element;
      const container = element;
      const text = clean(element.innerText);
      ${code}
      return { completed, progress };
    }`),
  ]),
];

function courseRow({ label = '', iconClass = '', iconStyle = '', svgHref = '', rowClass = '' } = {}) {
  const icon = {
    className: iconClass,
    tagName: svgHref ? 'svg' : 'i',
    innerHTML: '',
    getAttribute: (name) => ({ class: iconClass, style: iconStyle })[name] || null,
    querySelector: (selector) => selector === 'use' && svgHref
      ? { getAttribute: (name) => name === 'href' ? svgHref : null }
      : null,
  };
  const row = {
    innerText: `02 市场调研概念及宏观环境分析（下）\n14分钟\n${label}`,
    className: rowClass,
    tagName: 'li',
    getAttribute: () => null,
    querySelector: () => null,
    querySelectorAll: () => [icon],
    closest: () => row,
  };
  return row;
}

const cases = [
  // YS 学堂实际渲染：已完成的 li 带 completed，当前选中的章节还会带 active。
  // 对勾是没有语义类名的 img，必须识别行本身的 completed。
  ['YS当前已完成章节的active completed行', { label: '上次学习', rowClass: 'active completed' }, true, 100],
  ['YS未选中的completed行', { rowClass: 'completed' }, true, 100],
  ['YS当前未完成章节只有active', { label: '上次学习', rowClass: 'active' }, false, 0],
  ['选中对勾不能当成完成', { rowClass: 'active checked' }, false, 0],
  ['完成对勾与上次学习并存', { label: '上次学习', iconClass: 'icon-check-circle' }, true, 100],
  ['已完成文本与上次学习并存', { label: '已完成 上次学习' }, true, 100],
  ['100%进度与上次学习并存', { label: '进度：100% 上次学习' }, true, 100],
  ['选中的已完成章节', { label: '上次学习', rowClass: 'chapter-item active', svgHref: '#icon-check' }, true, 100],
  ['普通已完成章节', { iconClass: 'icon-check-circle' }, true, 100],
  ['仅有上次学习不能推断完成', { label: '上次学习' }, false, 0],
  ['未学习仍然不能完成', { label: '未学习 上次学习', iconClass: 'icon-check-circle' }, false, 0],
  ['学习中仍然不能完成', { label: '学习中 上次学习', iconClass: 'icon-check-circle' }, false, 0],
  ['橙色待学图标不能完成', { label: '上次学习', iconClass: 'icon-check-circle', iconStyle: 'color: orange' }, false, 0],
];

for (const [readerName, read] of readers) {
  for (const [name, fixture, completed, progress] of cases) {
    test(`${readerName}：${name}`, () => {
      const result = read(courseRow(fixture));
      if (typeof result === 'boolean') {
        assert.equal(result, completed);
      } else {
        assert.equal(result.completed, completed);
        assert.equal(result.progress, progress);
      }
    });
  }
}

for (const [readerName, read] of readers.slice(1)) {
  test(`${readerName}：上次学习保留部分进度`, () => {
    const result = read(courseRow({ label: '进度：42% 上次学习' }));
    assert.equal(result.completed, false);
    assert.equal(result.progress, 42);
  });
}
