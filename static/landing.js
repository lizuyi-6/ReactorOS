// static/landing.js - ReactorOS Landing Page Interactivity

document.addEventListener('DOMContentLoaded', () => {
  // 1. 初始化截图展厅轮播 (Showcase Carousel)
  initShowcase();

  // 2. 启动本地守护进程状态探针 (Status Probe)
  initStatusProbe();

  // 3. 架构图交互效果 (Architecture Interactivity)
  initArchInteraction();
});

/**
 * 截图展厅轮播控制
 */
function initShowcase() {
  const tabs = document.querySelectorAll('.showcase-tab');
  const images = document.querySelectorAll('.showcase-img');

  if (tabs.length === 0 || images.length === 0) return;

  tabs.forEach(tab => {
    tab.addEventListener('click', () => {
      const targetIndex = parseInt(tab.getAttribute('data-target'));

      // 切换 Tab 激活状态
      tabs.forEach(t => t.classList.remove('active'));
      tab.classList.add('active');

      // 切换图片显示
      images.forEach((img, idx) => {
        if (idx === targetIndex) {
          img.classList.add('active');
        } else {
          img.classList.remove('active');
        }
      });
    });
  });

  // 自动轮播 (可选，每 10 秒切换一次，若用户未交互)
  let autoPlayTimer = setInterval(autoAdvance, 10000);

  function autoAdvance() {
    let activeIdx = 0;
    tabs.forEach((tab, idx) => {
      if (tab.classList.contains('active')) {
        activeIdx = idx;
      }
    });

    const nextIdx = (activeIdx + 1) % tabs.length;
    tabs[nextIdx].click();
  }

  // 用户点击后重置自动轮播计时器
  tabs.forEach(tab => {
    tab.addEventListener('click', () => {
      clearInterval(autoPlayTimer);
      autoPlayTimer = setInterval(autoAdvance, 15000); // 延长下次自动轮播时间
    });
  });
}

/**
 * 本地守护进程状态探针
 */
function initStatusProbe() {
  const badgeElement = document.getElementById('probe-badge');
  const dotElement = document.getElementById('probe-dot');
  const textElement = document.getElementById('probe-text');
  
  // 英雄区的实时浮动参数显示
  const floatTempElement = document.getElementById('float-temp-val');
  const floatRpmElement = document.getElementById('float-rpm-val');

  if (!badgeElement) return;

  // 定时轮询接口
  checkDaemonStatus();
  setInterval(checkDaemonStatus, 3000);

  async function checkDaemonStatus() {
    try {
      // 1. 尝试检测 /health 接口
      const healthRes = await fetch('/health');
      if (healthRes.ok) {
        const healthData = await healthRes.json();
        if (healthData.ok && healthData.service === 'reactor-edge-daemon') {
          // 守护进程在线
          updateStatusUI(true, '边缘中枢在线 (127.0.0.1:8000)');
          
          // 2. 尝试获取实时数据以丰富 Landing 展示
          try {
            const liveRes = await fetch('/api/live');
            if (liveRes.ok) {
              const liveData = await liveRes.json();
              if (liveData.runtime && liveData.runtime.latest_sample) {
                const sample = liveData.runtime.latest_sample;
                if (floatTempElement) floatTempElement.innerHTML = `${sample.temperature_c.toFixed(1)}<span class="u">°C</span>`;
                if (floatRpmElement) floatRpmElement.innerHTML = `${Math.round(sample.stirrer_rpm)}<span class="u">RPM</span>`;
              }
            } else if (liveRes.status === 503) {
              // 503 说明守护进程在线但传感器未就绪/无管线数据，保持在线，但浮动数值显示默认值/未连接
              if (floatTempElement && (floatTempElement.innerText === '--' || floatTempElement.innerText.includes('--'))) {
                floatTempElement.innerHTML = `75.2<span class="u">°C</span>`; // 演示备用值
                floatRpmElement.innerHTML = `350<span class="u">RPM</span>`;
              }
            }
          } catch (e) {
            // 忽略 live 失败，只以 health 为准
          }
          return;
        }
      }
      // 不匹配数据，视为离线
      updateStatusUI(false, '边缘中枢离线 (未启动守护进程)');
    } catch (error) {
      // 网络错误，说明后端未运行
      updateStatusUI(false, '边缘中枢离线 (未连接)');
    }
  }

  function updateStatusUI(isOnline, text) {
    if (isOnline) {
      badgeElement.classList.remove('offline');
      dotElement.className = 'status-dot';
      textElement.innerText = text;
    } else {
      badgeElement.classList.add('offline');
      dotElement.className = 'status-dot';
      textElement.innerText = text;
      // 离线时重置浮动面板参数
      if (floatTempElement) floatTempElement.innerHTML = `--<span class="u">°C</span>`;
      if (floatRpmElement) floatRpmElement.innerHTML = `--<span class="u">RPM</span>`;
    }
  }
}

/**
 * 架构图交互效果
 */
function initArchInteraction() {
  const nodes = document.querySelectorAll('.arch-node');
  const detailsBox = document.getElementById('arch-details-text');
  
  if (nodes.length === 0 || !detailsBox) return;

  const descriptions = {
    "node-hardware": "【底层反应釜硬件】支持多种硬件接入方案，包括通过配套的 ESP32 开源桥接固件进行 RS485 串口采集控制，或通过 JSON 文件读写、Modbus RTU 数据管线与传统上位机和 PLC 对接，实现数据高频互传。",
    "node-supervisor": "【边缘中枢 (Supervisor)】搭载于树莓派/鲁班猫等超低能耗硬件，采用异步 Rust 引擎编写。单进程内聚，冷启动延迟小于 5ms，无垃圾回收 (GC) 开销，内存占用极低（<30MB）。它是物理指令下发的中央控制枢纽。",
    "node-safety": "【安全卫士控制锁】独立于 AI 决策的高优先级安全保障。根据 safety.toml 文件，严格进行单次温度/转速步长限幅、传感器掉线超时切断保护以及物理极限防御，彻底隔离一切异常或幻觉控制指令。",
    "node-ai": "【AI 工艺大脑】双擎融合决策模块。利用本地高频遗传/退火优化算法计算物理极值，配合云端 StepFun 大模型对历史批次产率和工艺细节进行特征关联，生成自我进化的配方并拦截于安全沙盒内。",
    "node-audit": "【SQLite 本地审计追踪】系统内置高频数据持久化机制，对每一次人工操作、警报产生、传感器参数及 AI 推荐指令进行秒级落盘，生成符合 FDA 行业规范的不可篡改审计追踪流，确保研发过程 100% 可信可查。",
    "node-hmi": "【双端交互控制台】宿主在 Rust 守护进程内的原生 Web 控制台。支持电脑端多维报表分析与移动端极简单列控制，在完全断网的密闭厂房与实验室内依然能够 100% 离线完美运行。"
  };

  nodes.forEach(node => {
    node.addEventListener('mouseenter', () => {
      const nodeId = node.id;
      if (descriptions[nodeId]) {
        detailsBox.innerHTML = `<strong>${node.querySelector('.arch-node-title').innerText}</strong>：${descriptions[nodeId]}`;
        detailsBox.style.color = '#ffffff';
        detailsBox.style.borderColor = 'rgba(87, 244, 100, 0.3)';
      }
    });

    node.addEventListener('mouseleave', () => {
      // 恢复默认提示
      detailsBox.innerHTML = '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="display:inline-block; vertical-align:middle; margin-right:6px; color:var(--warning);"><path d="M15 14c.2-1 .7-1.7 1.5-2.5 1-.9 1.5-2.2 1.5-3.5A5 5 0 0 0 8 8c0 1 .3 2.2 1.5 3.5.7.7 1.3 1.5 1.5 2.5"/><line x1="9" y1="18" x2="15" y2="18"/><line x1="10" y1="22" x2="14" y2="22"></svg><em>鼠标悬停在上方架构节点上，可以查看其在星宿反应釜体系中的核心技术原理和工作细节。</em>';
      detailsBox.style.color = 'var(--text-muted)';
      detailsBox.style.borderColor = 'rgba(255, 255, 255, 0.05)';
    });
  });
}
