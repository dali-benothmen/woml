# AgriMind - Smart Farming AI

Intelligent agricultural monitoring and optimization system

```typescript
import { cronflow } from 'cronflow';
import { z } from 'zod';

// Types
interface Field {
  id: string;
  name: string;
  cropType: string;
  area: number; // hectares
  location: { lat: number; lng: number };
  plantingDate: Date;
}

interface WeatherData {
  temperature: number;
  humidity: number;
  rainfall: number;
  windSpeed: number;
  forecast: Array<{
    date: string;
    temp: number;
    precipitation: number;
  }>;
}

interface SoilData {
  moisture: number; // percentage
  ph: number;
  nitrogen: number;
  phosphorus: number;
  potassium: number;
  temperature: number;
}

interface CropHealth {
  ndviScore: number; // 0-1 (vegetation index)
  healthStatus: 'excellent' | 'good' | 'fair' | 'poor';
  stressIndicators: string[];
  growthStage: string;
  estimatedYield: number;
}

interface IrrigationRecommendation {
  shouldIrrigate: boolean;
  duration: number; // minutes
  priority: 'low' | 'medium' | 'high' | 'critical';
  waterAmount: number; // liters
  estimatedCost: number;
  reasoning: string;
}

interface PestRisk {
  level: 'low' | 'medium' | 'high' | 'critical';
  pests: string[];
  recommendations: string[];
  sprayRecommended: boolean;
}

interface FieldAnalysisResult {
  fieldId: string;
  fieldName: string;
  cropType: string;
  weatherData: WeatherData;
  soilData: SoilData;
  cropHealth: CropHealth;
  marketPrices: { currentPrice: number; trend: string };
  irrigation: IrrigationRecommendation;
  pestRisk: PestRisk;
  analysis: {
    overallScore: number;
    profitProjection: {
      estimatedRevenue: number;
      estimatedCosts: number;
      profit: number;
    };
    riskFactors: string[];
  };
}

interface Recommendation {
  type: string;
  priority: string;
  action: string;
  reasoning: string;
}

interface WeatherAlert {
  fieldId: string;
  type: string;
  message: string;
  severity: string;
}

interface CriticalAlert {
  fieldId: string;
  fieldName: string;
  alerts: string[];
}

// Mock data functions (replace with real APIs)
async function getWeatherData(lat: number, lng: number): Promise<WeatherData> {
  // Mock weather API call
  return {
    temperature: 25 + Math.random() * 10,
    humidity: 60 + Math.random() * 30,
    rainfall: Math.random() * 20,
    windSpeed: 5 + Math.random() * 15,
    forecast: Array.from({ length: 7 }, (_, i) => ({
      date: new Date(Date.now() + i * 24 * 60 * 60 * 1000).toISOString(),
      temp: 20 + Math.random() * 15,
      precipitation: Math.random() * 15,
    })),
  };
}

async function getSoilData(fieldId: string): Promise<SoilData> {
  // Mock soil sensor data
  return {
    moisture: 30 + Math.random() * 40,
    ph: 6.0 + Math.random() * 2,
    nitrogen: 20 + Math.random() * 30,
    phosphorus: 15 + Math.random() * 20,
    potassium: 200 + Math.random() * 100,
    temperature: 18 + Math.random() * 12,
  };
}

async function analyzeCropHealth(
  fieldId: string,
  cropType: string
): Promise<CropHealth> {
  // Mock satellite/drone imagery analysis
  const ndviScore = 0.3 + Math.random() * 0.6;
  let healthStatus: CropHealth['healthStatus'] = 'good';

  if (ndviScore > 0.8) healthStatus = 'excellent';
  else if (ndviScore > 0.6) healthStatus = 'good';
  else if (ndviScore > 0.4) healthStatus = 'fair';
  else healthStatus = 'poor';

  const stressIndicators: string[] = [];
  if (ndviScore < 0.5) stressIndicators.push('Water stress');
  if (Math.random() > 0.8) stressIndicators.push('Nutrient deficiency');
  if (Math.random() > 0.9) stressIndicators.push('Disease detected');

  return {
    ndviScore,
    healthStatus,
    stressIndicators,
    growthStage: [
      'seedling',
      'vegetative',
      'flowering',
      'fruiting',
      'maturity',
    ][Math.floor(Math.random() * 5)],
    estimatedYield: 3000 + Math.random() * 2000, // kg per hectare
  };
}

async function getMarketPrices(
  cropType: string
): Promise<{ currentPrice: number; trend: string }> {
  // Mock market data
  return {
    currentPrice: 2.5 + Math.random() * 1.5, // price per kg
    trend: ['up', 'down', 'stable'][Math.floor(Math.random() * 3)],
  };
}

function calculateIrrigationNeeds(
  soilData: SoilData,
  weatherData: WeatherData,
  cropHealth: CropHealth,
  cropType: string
): IrrigationRecommendation {
  let shouldIrrigate = false;
  let priority: IrrigationRecommendation['priority'] = 'low';
  let duration = 0;
  let waterAmount = 0;
  let reasoning = '';

  // Soil moisture check
  const moistureThreshold =
    cropType === 'tomato' ? 70 : cropType === 'wheat' ? 50 : 60;

  if (soilData.moisture < moistureThreshold * 0.4) {
    shouldIrrigate = true;
    priority = 'critical';
    duration = 45;
    reasoning = 'Critical soil moisture level';
  } else if (soilData.moisture < moistureThreshold * 0.6) {
    shouldIrrigate = true;
    priority = 'high';
    duration = 30;
    reasoning = 'Low soil moisture';
  } else if (soilData.moisture < moistureThreshold * 0.8) {
    // Check weather forecast
    const upcomingRain = weatherData.forecast
      .slice(0, 3)
      .reduce((sum, day) => sum + day.precipitation, 0);

    if (upcomingRain < 10) {
      shouldIrrigate = true;
      priority = 'medium';
      duration = 20;
      reasoning = 'Moderate moisture with no rain forecast';
    }
  }

  // Factor in crop health
  if (cropHealth.stressIndicators.includes('Water stress')) {
    shouldIrrigate = true;
    priority = priority === 'low' ? 'high' : priority;
    duration = Math.max(duration, 25);
    reasoning += (reasoning ? ' + ' : '') + 'Crop showing water stress';
  }

  // Calculate water amount (rough estimate)
  waterAmount = duration * 50; // 50 liters per minute per hectare

  return {
    shouldIrrigate,
    duration,
    priority,
    waterAmount,
    estimatedCost: waterAmount * 0.002, // $0.002 per liter
    reasoning: reasoning || 'Adequate moisture levels',
  };
}

function assessPestRisk(
  weatherData: WeatherData,
  cropHealth: CropHealth,
  cropType: string,
  growthStage: string
): PestRisk {
  let riskLevel: PestRisk['level'] = 'low';
  const pests: string[] = [];
  const recommendations: string[] = [];

  // Weather-based risk factors
  if (weatherData.humidity > 80 && weatherData.temperature > 25) {
    riskLevel = 'high';
    pests.push('Fungal diseases', 'Aphids');
  }

  if (weatherData.temperature > 30 && weatherData.humidity < 40) {
    pests.push('Spider mites', 'Thrips');
    if (riskLevel === 'low') riskLevel = 'medium';
  }

  // Crop health factors
  if (cropHealth.stressIndicators.length > 0) {
    if (riskLevel === 'low') riskLevel = 'medium';
    else if (riskLevel === 'medium') riskLevel = 'high';
    pests.push('Opportunistic pests');
  }

  // Growth stage vulnerabilities
  if (growthStage === 'flowering' || growthStage === 'fruiting') {
    pests.push('Fruit flies', 'Caterpillars');
    if (riskLevel === 'low') riskLevel = 'medium';
  }

  // Critical conditions
  if (weatherData.humidity > 90 && weatherData.temperature > 30) {
    riskLevel = 'critical';
    pests.push('Severe fungal outbreak risk');
  }

  // Generate recommendations
  if (riskLevel === 'high' || riskLevel === 'critical') {
    recommendations.push('Increase monitoring frequency');
    recommendations.push('Consider preventive spray application');
  }

  if (pests.includes('Fungal diseases')) {
    recommendations.push('Improve air circulation');
    recommendations.push('Reduce leaf wetness');
  }

  return {
    level: riskLevel,
    pests: [...new Set(pests)], // Remove duplicates
    recommendations,
    sprayRecommended: riskLevel === 'high' || riskLevel === 'critical',
  };
}

// Mock field data
const fields: Field[] = [
  {
    id: 'field_001',
    name: 'North Field',
    cropType: 'tomato',
    area: 2.5,
    location: { lat: 40.7128, lng: -74.006 },
    plantingDate: new Date('2024-03-15'),
  },
  {
    id: 'field_002',
    name: 'South Field',
    cropType: 'wheat',
    area: 5.0,
    location: { lat: 40.758, lng: -73.9855 },
    plantingDate: new Date('2024-02-01'),
  },
];

// Schema for webhooks
const fieldAnalysisSchema = z.object({
  fieldId: z.string(),
  forceUpdate: z.boolean().optional(),
});

const irrigationControlSchema = z.object({
  fieldId: z.string(),
  action: z.enum(['start', 'stop', 'schedule']),
  duration: z.number().optional(),
  scheduleTime: z.string().optional(),
});

// Main AgriMind Workflow
const agriMindWorkflow = cronflow.define({
  id: 'agrimind-smart-farming',
  name: 'AgriMind - Smart Farming AI',
  description: 'Intelligent agricultural monitoring and optimization system',
  hooks: {
    onSuccess: (ctx, stepId) => {
      if (!stepId) {
        console.log('🌱 AgriMind analysis completed successfully!');
      }
    },
    onFailure: (ctx, stepId) => {
      console.error(`❌ AgriMind failed at step ${stepId}:`, ctx.step_error);
    },
  },
});

// Scheduled Field Analysis (every 6 hours)
agriMindWorkflow
  .onSchedule('0 */6 * * *')
  .step('analyze-all-fields', async ctx => {
    console.log('🚜 Starting scheduled field analysis...');

    const analysisResults: FieldAnalysisResult[] = [];

    for (const field of fields) {
      const result = await analyzeField(field);
      analysisResults.push(result);
    }

    return {
      timestamp: new Date().toISOString(),
      fieldsAnalyzed: analysisResults.length,
      results: analysisResults,
    };
  })
  .step('process-critical-alerts', async ctx => {
    const criticalFields = ctx.last.results.filter(
      (result: FieldAnalysisResult) =>
        result.irrigation.priority === 'critical' ||
        result.pestRisk.level === 'critical'
    );

    const alerts: CriticalAlert[] = criticalFields.map(
      (field: FieldAnalysisResult) => ({
        fieldId: field.fieldId,
        fieldName: field.fieldName,
        alerts: [
          ...(field.irrigation.priority === 'critical'
            ? ['Critical irrigation needed']
            : []),
          ...(field.pestRisk.level === 'critical'
            ? ['Critical pest risk']
            : []),
        ],
      })
    );

    if (alerts.length > 0) {
      console.log('🚨 CRITICAL ALERTS:', alerts);
    }

    return { criticalAlerts: alerts };
  })
  .if('has-critical-alerts', ctx => ctx.last.criticalAlerts.length > 0)
  .step('handle-critical-alerts', async ctx => {
    const results = await Promise.all(
      ctx.last.criticalAlerts.map((alert: CriticalAlert) =>
        handleCriticalAlert(alert)
      )
    );
    return { alertResults: results };
  })
  .endIf()
  .action('log-analysis-summary', ctx => {
    const { fieldsAnalyzed, criticalAlerts } = ctx.last;
    console.log(
      `📊 Analysis Summary: ${fieldsAnalyzed} fields analyzed, ${criticalAlerts?.length || 0} critical alerts`
    );
  });

// Single Field Analysis Webhook
agriMindWorkflow
  .onWebhook('/webhooks/analyze-field', {
    schema: fieldAnalysisSchema,
  })
  .step('single-field-analysis', async ctx => {
    const { fieldId, forceUpdate } = ctx.payload;

    const field = fields.find(f => f.id === fieldId);
    if (!field) {
      throw new Error(`Field not found: ${fieldId}`);
    }

    console.log(`🔍 Analyzing field: ${field.name} (${field.cropType})`);

    const result = await analyzeField(field);

    return {
      fieldId,
      analysis: result,
      timestamp: new Date().toISOString(),
    };
  })
  .step('generate-recommendations', async ctx => {
    const { analysis } = ctx.last;

    const recommendations: Recommendation[] = [];

    // Irrigation recommendations
    if (analysis.irrigation.shouldIrrigate) {
      recommendations.push({
        type: 'irrigation',
        priority: analysis.irrigation.priority,
        action: `Irrigate for ${analysis.irrigation.duration} minutes`,
        reasoning: analysis.irrigation.reasoning,
      });
    }

    // Pest management
    if (analysis.pestRisk.sprayRecommended) {
      recommendations.push({
        type: 'pest_control',
        priority: analysis.pestRisk.level,
        action: 'Apply pest control spray',
        reasoning: `Pest risk: ${analysis.pestRisk.pests.join(', ')}`,
      });
    }

    // Fertilizer recommendations based on soil data
    if (analysis.soilData.nitrogen < 25) {
      recommendations.push({
        type: 'fertilizer',
        priority: analysis.soilData.nitrogen < 15 ? 'high' : 'medium',
        action: 'Apply nitrogen fertilizer',
        reasoning: `Low nitrogen levels: ${analysis.soilData.nitrogen}`,
      });
    }

    return {
      fieldId: ctx.last.fieldId,
      recommendations,
      totalRecommendations: recommendations.length,
    };
  })
  .action('notify-farmer', ctx => {
    const { fieldId, recommendations } = ctx.last;
    const field = fields.find(f => f.id === fieldId);

    console.log(
      `📱 Sending ${recommendations.length} recommendations for ${field?.name}`
    );
    // Here you could send SMS, email, or app notifications
  });

// Irrigation Control Webhook
agriMindWorkflow
  .onWebhook('/webhooks/irrigation-control', {
    schema: irrigationControlSchema,
  })
  .step('handle-irrigation-command', async ctx => {
    const { fieldId, action, duration, scheduleTime } = ctx.payload;

    const field = fields.find(f => f.id === fieldId);
    if (!field) {
      throw new Error(`Field not found: ${fieldId}`);
    }

    console.log(`💧 Irrigation ${action} for ${field.name}`);

    switch (action) {
      case 'start':
        // Mock irrigation system control
        console.log(`🌊 Starting irrigation for ${duration || 30} minutes`);
        return {
          fieldId,
          action: 'started',
          duration: duration || 30,
          estimatedCost: (duration || 30) * 50 * 0.002,
        };

      case 'stop':
        console.log(`🛑 Stopping irrigation`);
        return {
          fieldId,
          action: 'stopped',
        };

      case 'schedule':
        console.log(`⏰ Scheduling irrigation for ${scheduleTime}`);
        return {
          fieldId,
          action: 'scheduled',
          scheduleTime,
          duration: duration || 30,
        };

      default:
        throw new Error(`Unknown irrigation action: ${action}`);
    }
  })
  .action('log-irrigation-action', ctx => {
    const { fieldId, action } = ctx.last;
    const field = fields.find(f => f.id === fieldId);
    console.log(`✅ Irrigation ${action} completed for ${field?.name}`);
  });

// Weather Alert Workflow
const weatherAlertWorkflow = cronflow.define({
  id: 'weather-alerts',
  name: 'Weather Alert System',
  description: 'Monitor weather conditions and send alerts',
});

weatherAlertWorkflow
  .onSchedule('0 */2 * * *') // Every 2 hours
  .step('check-weather-alerts', async ctx => {
    const alerts: WeatherAlert[] = [];

    for (const field of fields) {
      const weather = await getWeatherData(
        field.location.lat,
        field.location.lng
      );

      // Check for extreme conditions
      if (weather.temperature > 35) {
        alerts.push({
          fieldId: field.id,
          type: 'high_temperature',
          message: `High temperature alert: ${weather.temperature}°C`,
          severity: 'high',
        });
      }

      if (weather.windSpeed > 25) {
        alerts.push({
          fieldId: field.id,
          type: 'high_wind',
          message: `High wind speed: ${weather.windSpeed} km/h`,
          severity: 'medium',
        });
      }

      // Check upcoming heavy rain
      const heavyRain = weather.forecast.find(day => day.precipitation > 50);
      if (heavyRain) {
        alerts.push({
          fieldId: field.id,
          type: 'heavy_rain',
          message: `Heavy rain expected: ${heavyRain.precipitation}mm on ${heavyRain.date}`,
          severity: 'medium',
        });
      }
    }

    return { alerts, timestamp: new Date().toISOString() };
  })
  .if('has-weather-alerts', ctx => ctx.last.alerts.length > 0)
  .step('process-weather-alerts', async ctx => {
    const { alerts } = ctx.last;

    console.log(`🌦️ Weather alerts generated: ${alerts.length}`);
    alerts.forEach((alert: WeatherAlert) => {
      const field = fields.find(f => f.id === alert.fieldId);
      console.log(`⚠️ ${field?.name}: ${alert.message}`);
    });

    return { alertsProcessed: alerts.length };
  })
  .endIf();

// Helper Functions
async function analyzeField(field: Field): Promise<FieldAnalysisResult> {
  console.log(`📊 Analyzing ${field.name}...`);

  // Gather all data in parallel for speed
  const [weatherData, soilData, cropHealth, marketPrices] = await Promise.all([
    getWeatherData(field.location.lat, field.location.lng),
    getSoilData(field.id),
    analyzeCropHealth(field.id, field.cropType),
    getMarketPrices(field.cropType),
  ]);

  // Calculate recommendations
  const irrigation = calculateIrrigationNeeds(
    soilData,
    weatherData,
    cropHealth,
    field.cropType
  );
  const pestRisk = assessPestRisk(
    weatherData,
    cropHealth,
    field.cropType,
    cropHealth.growthStage
  );

  return {
    fieldId: field.id,
    fieldName: field.name,
    cropType: field.cropType,
    weatherData,
    soilData,
    cropHealth,
    marketPrices,
    irrigation,
    pestRisk,
    analysis: {
      overallScore: calculateOverallScore(
        cropHealth,
        soilData,
        irrigation,
        pestRisk
      ),
      profitProjection: calculateProfitProjection(
        field,
        cropHealth,
        marketPrices,
        irrigation
      ),
      riskFactors: identifyRiskFactors(
        weatherData,
        soilData,
        cropHealth,
        pestRisk
      ),
    },
  };
}

async function handleCriticalAlert(alert: CriticalAlert) {
  console.log(`🚨 Handling critical alert for ${alert.fieldName}`);

  // Auto-actions for critical situations
  if (alert.alerts.includes('Critical irrigation needed')) {
    console.log(`💧 Auto-starting emergency irrigation for ${alert.fieldName}`);
    // Could trigger actual irrigation system here
  }

  if (alert.alerts.includes('Critical pest risk')) {
    console.log(`🐛 Scheduling emergency pest control for ${alert.fieldName}`);
    // Could schedule pest control service
  }

  return {
    fieldId: alert.fieldId,
    actionsTriggered: alert.alerts.length,
    handled: true,
  };
}

function calculateOverallScore(
  cropHealth: CropHealth,
  soilData: SoilData,
  irrigation: IrrigationRecommendation,
  pestRisk: PestRisk
): number {
  let score = cropHealth.ndviScore * 40; // 40% weight

  // Soil health score (30% weight)
  const soilScore = Math.min(
    100,
    (soilData.moisture / 80 + soilData.ph / 8 + soilData.nitrogen / 50) * 33.3
  );
  score += soilScore * 0.3;

  // Water management score (20% weight)
  const waterScore =
    irrigation.priority === 'critical'
      ? 20
      : irrigation.priority === 'high'
        ? 60
        : 90;
  score += waterScore * 0.2;

  // Pest risk score (10% weight)
  const pestScore =
    pestRisk.level === 'critical'
      ? 20
      : pestRisk.level === 'high'
        ? 50
        : pestRisk.level === 'medium'
          ? 75
          : 90;
  score += pestScore * 0.1;

  return Math.round(score);
}

function calculateProfitProjection(
  field: Field,
  cropHealth: CropHealth,
  marketPrices: { currentPrice: number; trend: string },
  irrigation: IrrigationRecommendation
): { estimatedRevenue: number; estimatedCosts: number; profit: number } {
  const estimatedYield = cropHealth.estimatedYield * field.area;
  const estimatedRevenue = estimatedYield * marketPrices.currentPrice;

  // Basic cost estimation
  const estimatedCosts = field.area * 1000 + irrigation.estimatedCost * 30; // Monthly irrigation

  return {
    estimatedRevenue,
    estimatedCosts,
    profit: estimatedRevenue - estimatedCosts,
  };
}

function identifyRiskFactors(
  weather: WeatherData,
  soil: SoilData,
  cropHealth: CropHealth,
  pestRisk: PestRisk
): string[] {
  const risks: string[] = [];

  if (weather.temperature > 35) risks.push('Extreme heat stress');
  if (weather.rainfall < 5) risks.push('Drought conditions');
  if (soil.moisture < 30) risks.push('Low soil moisture');
  if (soil.ph < 6 || soil.ph > 8) risks.push('Soil pH imbalance');
  if (cropHealth.healthStatus === 'poor') risks.push('Poor crop health');
  if (pestRisk.level === 'high' || pestRisk.level === 'critical')
    risks.push('High pest pressure');

  return risks;
}

console.log('🌱 AgriMind Smart Farming AI Starting...');
console.log('📡 Monitoring agricultural data every 6 hours');
console.log('🎯 Available endpoints:');
console.log('   POST /webhooks/analyze-field - {"fieldId": "field_001"}');
console.log(
  '   POST /webhooks/irrigation-control - {"fieldId": "field_001", "action": "start", "duration": 30}'
);
console.log('');
console.log('🚜 Monitoring fields:');
fields.forEach(field => {
  console.log(`   - ${field.name}: ${field.cropType} (${field.area} hectares)`);
});
console.log('');
console.log('🧠 AI Capabilities:');
console.log('   ✅ Satellite/Drone imagery analysis');
console.log('   ✅ Weather correlation & forecasting');
console.log('   ✅ Soil sensor integration');
console.log('   ✅ Intelligent irrigation optimization');
console.log('   ✅ Pest risk prediction');
console.log('   ✅ Market price integration');
console.log('   ✅ Automated critical alerts');

export { agriMindWorkflow, weatherAlertWorkflow, fields };
```
