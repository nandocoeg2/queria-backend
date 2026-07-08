import React, { useRef, useEffect, useMemo } from 'react';
import * as THREE from 'three';

export interface GraphNode {
  id: string;
  type: 'document' | 'knowledge_item';
  name: string;
  x: number;
  y: number;
  z: number;
  color: string;
  size: number;
  score?: number;
}

interface NodeCloudProps {
  nodes: GraphNode[];
  hoveredId: string | null;
  selectedId: string | null;
  onHover: (id: string | null, e?: any) => void;
  onClick: (id: string) => void;
}

export default function NodeCloud({
  nodes,
  hoveredId,
  selectedId,
  onHover,
  onClick,
}: NodeCloudProps) {
  const meshRef = useRef<THREE.InstancedMesh>(null);

  // Dynamic LOD (tessellation) of the sphere geometry based on node count
  const sphereGeometry = useMemo(() => {
    const count = nodes.length;
    let widthSegs = 16;
    let heightSegs = 12;

    if (count <= 1000) {
      widthSegs = 32;
      heightSegs = 24;
    } else if (count > 25000) {
      widthSegs = 10;
      heightSegs = 7;
    }

    return new THREE.SphereGeometry(1, widthSegs, heightSegs);
  }, [nodes.length]);

  // Handle setting instance matrices and colors when nodes, hoveredId, or selectedId change
  useEffect(() => {
    const mesh = meshRef.current;
    if (!mesh) return;

    const tempObject = new THREE.Object3D();
    const tempColor = new THREE.Color();

    nodes.forEach((node, i) => {
      tempObject.position.set(node.x, node.y, node.z);

      // Determine size/scale: highlight hovered/selected nodes
      let scale = node.size;
      const isSelected = selectedId === node.id;
      const isHovered = hoveredId === node.id;

      if (isSelected) {
        scale = node.size * 1.5;
      } else if (isHovered) {
        scale = node.size * 1.3;
      } else if (hoveredId || selectedId) {
        // Dim other nodes when there's an active highlight
        scale = node.size * 0.7;
      }
      tempObject.scale.set(scale, scale, scale);
      tempObject.updateMatrix();
      mesh.setMatrixAt(i, tempObject.matrix);

      // Node color and bloom boost
      let baseColorStr = node.color;
      
      // If there's a selection or hover, fade out non-highlighted nodes
      let opacity = 1.0;
      if (hoveredId || selectedId) {
        const isHighlighted = isSelected || isHovered;
        if (!isHighlighted) {
          opacity = 0.2;
        }
      }

      tempColor.set(baseColorStr);
      
      // Boost color intensity above 1.0 for the Bloom effect (glowing look)
      if (isSelected || isHovered) {
        tempColor.multiplyScalar(2.5); // make it shine bright!
      } else {
        tempColor.multiplyScalar(opacity);
      }

      mesh.setColorAt(i, tempColor);
    });

    mesh.instanceMatrix.needsUpdate = true;
    if (mesh.instanceColor) {
      mesh.instanceColor.needsUpdate = true;
    }
  }, [nodes, hoveredId, selectedId]);

  // Handle Raycasting pointer events
  const handlePointerOver = (e: any) => {
    e.stopPropagation();
    if (e.instanceId !== undefined && nodes[e.instanceId]) {
      document.body.style.cursor = 'pointer';
      onHover(nodes[e.instanceId].id, e);
    }
  };

  const handlePointerOut = (e: any) => {
    e.stopPropagation();
    document.body.style.cursor = 'default';
    onHover(null);
  };

  const handlePointerClick = (e: any) => {
    e.stopPropagation();
    if (e.instanceId !== undefined && nodes[e.instanceId]) {
      onClick(nodes[e.instanceId].id);
    }
  };

  return (
    <instancedMesh
      ref={meshRef}
      args={[sphereGeometry, null as any, nodes.length]}
      onPointerOver={handlePointerOver}
      onPointerOut={handlePointerOut}
      onClick={handlePointerClick}
    >
      <meshStandardMaterial
        roughness={0.1}
        metalness={0.9}
        emissive={new THREE.Color('#221a14')}
        emissiveIntensity={0.2}
      />
    </instancedMesh>
  );
}
