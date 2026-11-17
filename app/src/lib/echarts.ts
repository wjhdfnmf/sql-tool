// The parts of Apache ECharts the graph builder draws with, and nothing
// else. This file is loaded only when a graph is first opened, so the chart
// library costs nothing at startup or for people who never open one.

import { BarChart, BoxplotChart, CustomChart, LineChart, PieChart, ScatterChart } from 'echarts/charts';
import { GridComponent, LegendComponent, MarkLineComponent, TitleComponent, TooltipComponent } from 'echarts/components';
import * as echarts from 'echarts/core';
import { CanvasRenderer } from 'echarts/renderers';

echarts.use([
  BarChart,
  LineChart,
  PieChart,
  ScatterChart,
  BoxplotChart,
  CustomChart,
  GridComponent,
  LegendComponent,
  MarkLineComponent,
  TitleComponent,
  TooltipComponent,
  CanvasRenderer,
]);

export const init = echarts.init;
export type Chart = ReturnType<typeof echarts.init>;
export type ChartOption = Parameters<Chart['setOption']>[0];
