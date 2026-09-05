// ECharts 按需引入(LubanCat2 / RK3568 低载专项优化)。
// 项目只用到折线图 + 网格/图例/提示框/标线 + Canvas 渲染;按需注册比
// `import * as echarts from "echarts"` 整包小数百 KB,直接减小 kiosk
// 单文件 HMI 的下载与 A55 上的 JS 解析量。新增图表类型时在此补注册。
import * as echarts from "echarts/core";
import { LineChart } from "echarts/charts";
import {
  GridComponent,
  LegendComponent,
  MarkLineComponent,
  TooltipComponent
} from "echarts/components";
import { CanvasRenderer } from "echarts/renderers";

echarts.use([
  LineChart,
  GridComponent,
  LegendComponent,
  MarkLineComponent,
  TooltipComponent,
  CanvasRenderer
]);

export default echarts;
export type { EChartsType } from "echarts/core";
