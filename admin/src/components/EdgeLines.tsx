import React, { useMemo } from 'react';
import * as THREE from 'three';
import type { GraphNode } from './NodeCloud';

export interface GraphEdge {
  source: string;
  target: string;
  type: 'has_knowledge' | 'semantic_relation';
  color: string;
}

interface EdgeLinesProps {
  edges: GraphEdge[];
  nodesMap: Map<string, GraphNode>;
  hoveredId: string | null;
  selectedId: string | null;
}

export default function EdgeLines({
  edges,
  nodesMap,
  hoveredId,
  selectedId,
}: EdgeLinesProps) {
  
  // Compute line position and color attribute buffers
  const [positions, colors] = useMemo(() => {
    const activeEdges: GraphEdge[] = [];
    
    // Filter active edges and handle highlights
    edges.forEach((edge) => {
      const sourceNode = nodesMap.get(edge.source);
      const targetNode = nodesMap.get(edge.target);
      if (sourceNode && targetNode) {
        activeEdges.push(edge);
      }
    });

    const posArray = new Float32Array(activeEdges.length * 6); // 2 points * 3 coords
    const colorArray = new Float32Array(activeEdges.length * 6); // 2 points * 3 colors

    const tempColor = new THREE.Color();

    activeEdges.forEach((edge, i) => {
      const sourceNode = nodesMap.get(edge.source)!;
      const targetNode = nodesMap.get(edge.target)!;

      // Source vertex position
      posArray[i * 6] = sourceNode.x;
      posArray[i * 6 + 1] = sourceNode.y;
      posArray[i * 6 + 2] = sourceNode.z;

      // Target vertex position
      posArray[i * 6 + 3] = targetNode.x;
      posArray[i * 6 + 4] = targetNode.y;
      posArray[i * 6 + 5] = targetNode.z;

      // Determine edge brightness based on highlight status
      let intensity = 0.15;
      const isSourceSelected = selectedId === edge.source;
      const isTargetSelected = selectedId === edge.target;
      const isSourceHovered = hoveredId === edge.source;
      const isTargetHovered = hoveredId === edge.target;

      if (selectedId || hoveredId) {
        if (isSourceSelected || isTargetSelected || isSourceHovered || isTargetHovered) {
          // Brighten relevant edges
          intensity = 0.8;
        } else {
          // Dim all other edges
          intensity = 0.02;
        }
      } else {
        // Subtle transparency for semantic relations vs hierarchical
        intensity = edge.type === 'semantic_relation' ? 0.08 : 0.2;
      }

      // Set source vertex color
      tempColor.set(edge.color).multiplyScalar(intensity);
      colorArray[i * 6] = tempColor.r;
      colorArray[i * 6 + 1] = tempColor.g;
      colorArray[i * 6 + 2] = tempColor.b;

      // Set target vertex color
      colorArray[i * 6 + 3] = tempColor.r;
      colorArray[i * 6 + 4] = tempColor.g;
      colorArray[i * 6 + 5] = tempColor.b;
    });

    return [posArray, colorArray];
  }, [edges, nodesMap, hoveredId, selectedId]);

  if (positions.length === 0) return null;

  return (
    <lineSegments>
      <bufferGeometry>
        <bufferAttribute
          attach="attributes-position"
          args={[positions, 3]}
        />
        <bufferAttribute
          attach="attributes-color"
          args={[colors, 3]}
        />
      </bufferGeometry>
      <lineBasicMaterial
        vertexColors
        transparent
        blending={THREE.AdditiveBlending}
        depthWrite={false}
        linewidth={1}
      />
    </lineSegments>
  );
}
